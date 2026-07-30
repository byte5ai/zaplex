use anyhow::Result;
use base64::Engine;
use flate2::read::ZlibDecoder;
use pathfinder_geometry::vector::Vector2F;
use rand::Rng;
use std::cmp::min;
use std::collections::HashMap;
use std::io::Read;
#[cfg(all(feature = "local_fs", unix))]
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
#[cfg(feature = "local_fs")]
use std::{env, fs, str};
use warpui::image_cache::{resize_dimensions, FitType};
use warpui::{
    assets::asset_cache::Asset,
    image_cache::{CustomHeaderCreationError, CustomImageFormat, CustomImageHeader, ImageType},
    util::{parse_i32, parse_u32},
};

use super::escape_sequences::C1;

/// Actions specified by the [Kitty Image Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
#[derive(Debug, Clone)]
pub enum KittyAction {
    StoreOnly(StoreOnly),
    StoreAndDisplay(StoreAndDisplay),
    DisplayStoredImage(DisplayStoredImage),
    QuerySupport(QuerySupport),
    Delete {
        delete_placements_only: bool,
        deletion_type: DeletionType,
    },
}

#[derive(Debug, Default, Clone)]
pub struct StoreOnly {
    pub image: KittyImage,
    pub image_id: u32,
}

#[derive(Debug, Default, Clone)]
pub struct StoreAndDisplay {
    pub image: KittyImage,
    pub placement_data: KittyPlacementData,
    pub image_id: u32,
    pub placement_id: u32,
}

#[derive(Debug, Default, Clone)]
pub struct DisplayStoredImage {
    pub placement_data: KittyPlacementData,
    pub image_id: u32,
    pub placement_id: u32,
}

#[derive(Debug, Default, Clone)]
pub struct QuerySupport {
    pub image: KittyImage,
    pub image_id: u32,
}

/// Which placements a `a=d` message asks the terminal to remove, as selected by
/// the message's `d=` key.
#[derive(Debug, Clone)]
pub enum DeletionType {
    /// `d=a`: every placement on screen.
    All,
    /// `d=i`: every placement of an image id, or a single placement when
    /// `placement_id` is given.
    ById {
        image_id: u32,
        placement_id: Option<u32>,
    },
    /// `d=n`: like [`DeletionType::ById`], but the image is selected by the
    /// client assigned image number (`I=`) rather than its id.
    ByNumber {
        image_number: u32,
        placement_id: Option<u32>,
    },
    /// `d=c`: every placement intersecting the cell the cursor is on.
    AtCursor,
    /// `d=p`: every placement intersecting the given cell.
    AtPoint { col: u32, row: u32 },
    /// `d=q`: every placement intersecting the given cell that also has the
    /// given z-index.
    AtPointZ { col: u32, row: u32, z: i32 },
    /// `d=x`: every placement intersecting the given column.
    Column(u32),
    /// `d=y`: every placement intersecting the given row.
    Row(u32),
    /// `d=z`: every placement with the given z-index.
    ZIndex(i32),
    /// `d=r`: every placement whose image id lies in the inclusive range.
    IdRange { start: u32, end: u32 },
    /// `d=f`: the animation frames of an image. Frames are not stored yet, so
    /// this is reported as unsupported.
    Frames { image_id: u32 },
}

#[derive(Debug, Default, Clone)]
pub struct KittyImage {
    pub metadata: KittyImageMetadata,
    pub data: Vec<u8>,
}

impl TryFrom<PendingKittyMessage> for KittyMessage {
    type Error = InvalidKittyPayload;

    fn try_from(pending: PendingKittyMessage) -> Result<KittyMessage, InvalidKittyPayload> {
        let mut decoded_payload = vec![];

        for payload in pending.payload {
            decoded_payload.extend(base64_decode_padding_agnostic(&payload[..])?);
        }

        Ok(KittyMessage {
            control_data: pending.control_data,
            payload: decoded_payload,
        })
    }
}

impl TryFrom<KittyMessage> for KittyImage {
    type Error = InvalidKittyPayload;

    fn try_from(message: KittyMessage) -> Result<Self, InvalidKittyPayload> {
        let decoded_data =
            decode_kitty_image_data(message.payload, message.control_data.compressed)?;

        let decoded_data = match message.control_data.transmission_medium {
            KittyTransmissionMedium::Direct => decoded_data,
            KittyTransmissionMedium::SimpleFile => {
                cfg_if::cfg_if! {
                    if #[cfg(feature = "local_fs")] {
                        read_file(decoded_data, false)?
                    } else {
                        return Err(InvalidKittyPayload::FileError(FileError::UnsupportedPlatform));
                    }
                }
            }
            KittyTransmissionMedium::TemporaryFile => {
                cfg_if::cfg_if! {
                    if #[cfg(feature = "local_fs")] {
                        read_file(decoded_data, true)?
                    } else {
                        return Err(InvalidKittyPayload::FileError(FileError::UnsupportedPlatform));
                    }
                }
            }
            KittyTransmissionMedium::SharedMemoryObject => {
                cfg_if::cfg_if! {
                    if #[cfg(all(feature = "local_fs", unix))] {
                        read_shared_memory(message.control_data.clone(), decoded_data)?
                    } else {
                        return Err(InvalidKittyPayload::ShmError(ShmError::UnsupportedPlatform));
                    }
                }
            }
        };

        Ok(Self {
            metadata: KittyImageMetadata::from(message.control_data),
            data: decoded_data,
        })
    }
}

pub type KittyResponse = Result<(), KittyError>;

#[derive(Debug, Clone)]
pub enum KittyError {
    InvalidKittyAction(InvalidKittyAction),
    StorageError(StorageError),
    KittyFeatureDisabled,
}

#[derive(Debug, Clone)]
pub enum StorageError {
    UnknownId { id: u32 },
}

impl From<StorageError> for KittyError {
    fn from(value: StorageError) -> Self {
        KittyError::StorageError(value)
    }
}

#[derive(Debug, Clone)]
pub enum InvalidKittyAction {
    UnsupportedAction,
    InvalidKittyPayload(InvalidKittyPayload),
    InvalidControlData(InvalidControlData),
}

impl From<InvalidKittyAction> for KittyError {
    fn from(value: InvalidKittyAction) -> Self {
        KittyError::InvalidKittyAction(value)
    }
}

#[derive(Debug, Clone)]
pub enum InvalidControlData {
    IdMissing,
    UnknownDeleteAction,
}

impl From<InvalidControlData> for KittyError {
    fn from(value: InvalidControlData) -> Self {
        KittyError::InvalidKittyAction(InvalidKittyAction::InvalidControlData(value))
    }
}

#[derive(Debug, Clone)]
pub enum InvalidKittyPayload {
    InvalidTransmissionMedium(KittyTransmissionMedium),
    KittyDecodeError(KittyDecodeError),
    InvalidKittyImage(InvalidKittyImage),
    FileError(FileError),
    ShmError(ShmError),
}

#[derive(Debug, Clone)]
pub enum FileError {
    FileReadError(String),
    UnsupportedPlatform,
}

impl From<InvalidKittyPayload> for KittyError {
    fn from(value: InvalidKittyPayload) -> Self {
        KittyError::InvalidKittyAction(InvalidKittyAction::InvalidKittyPayload(value))
    }
}

#[derive(Debug, Clone)]
pub enum ShmError {
    ObjectOpenError(String),
    MmapError,
    EmptyObject,
    ObjectTooSmall {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    InvalidObjectSize,
    FileStatError(String),
    UnsupportedPlatform,
}

#[derive(Debug, Clone)]
pub enum KittyDecodeError {
    InvalidBase64(String),
    InvalidCompression(String),
    InvalidUtf8(String),
}

impl From<KittyDecodeError> for InvalidKittyPayload {
    fn from(value: KittyDecodeError) -> Self {
        InvalidKittyPayload::KittyDecodeError(value)
    }
}

impl From<KittyDecodeError> for KittyError {
    fn from(value: KittyDecodeError) -> Self {
        KittyError::InvalidKittyAction(InvalidKittyAction::InvalidKittyPayload(
            InvalidKittyPayload::KittyDecodeError(value),
        ))
    }
}

#[derive(Debug, Clone)]
pub enum InvalidKittyImage {
    KittyPngError(KittyPngError),
    KittyRgbError(KittyRgbError),
    MalformedImage,
}

impl From<InvalidKittyImage> for KittyError {
    fn from(value: InvalidKittyImage) -> Self {
        KittyError::InvalidKittyAction(InvalidKittyAction::InvalidKittyPayload(
            InvalidKittyPayload::InvalidKittyImage(value),
        ))
    }
}

#[derive(Debug, Clone)]
pub enum KittyPngError {
    InvalidBytes(String),
}

impl From<KittyPngError> for KittyError {
    fn from(value: KittyPngError) -> Self {
        KittyError::InvalidKittyAction(InvalidKittyAction::InvalidKittyPayload(
            InvalidKittyPayload::InvalidKittyImage(InvalidKittyImage::KittyPngError(value)),
        ))
    }
}

#[derive(Debug, Clone)]
pub enum KittyRgbError {
    ExpectedDataSizeMismatch {
        expected_bytes: usize,
        actual_bytes: usize,
    },
}

impl From<KittyRgbError> for KittyError {
    fn from(value: KittyRgbError) -> Self {
        KittyError::InvalidKittyAction(InvalidKittyAction::InvalidKittyPayload(
            InvalidKittyPayload::InvalidKittyImage(InvalidKittyImage::KittyRgbError(value)),
        ))
    }
}

impl From<CustomHeaderCreationError> for KittyRgbError {
    fn from(value: CustomHeaderCreationError) -> Self {
        match value {
            CustomHeaderCreationError::ExpectedDataSizeMismatch {
                expected_bytes,
                actual_bytes,
            } => KittyRgbError::ExpectedDataSizeMismatch {
                expected_bytes,
                actual_bytes,
            },
        }
    }
}

impl TryFrom<KittyMessage> for KittyAction {
    type Error = KittyError;

    fn try_from(message: KittyMessage) -> Result<KittyAction, KittyError> {
        match message.control_data.placement_action {
            KittyPlacementAction::StoreOnly => {
                let mut action = StoreOnly {
                    image_id: message
                        .control_data
                        .image_id
                        .unwrap_or(rand::thread_rng().gen()),
                    image: KittyImage::try_from(message)?,
                };

                if action.image.metadata.pixel_data_format == KittyPixelDataFormat::Png {
                    action.image = set_kitty_png_size(action.image)?;
                } else {
                    action.image = set_kitty_rgb_headers(action.image)?;
                }

                Ok(KittyAction::StoreOnly(action))
            }
            KittyPlacementAction::StoreAndDisplay => {
                let mut action = StoreAndDisplay {
                    image_id: message
                        .control_data
                        .image_id
                        .unwrap_or(rand::thread_rng().gen()),
                    placement_id: message
                        .control_data
                        .placement_id
                        .unwrap_or(rand::thread_rng().gen()),
                    placement_data: KittyPlacementData {
                        z_index: message.control_data.z_index,
                        cols: message.control_data.cols,
                        rows: message.control_data.rows,
                        cursor_movement_policy: message.control_data.cursor_movement_policy,
                        unicode_placeholder: message.control_data.unicode_placeholder,
                    },
                    image: KittyImage::try_from(message)?,
                };

                if action.image.metadata.pixel_data_format == KittyPixelDataFormat::Png {
                    action.image = set_kitty_png_size(action.image)?;
                } else {
                    action.image = set_kitty_rgb_headers(action.image)?;
                }

                Ok(KittyAction::StoreAndDisplay(action))
            }
            KittyPlacementAction::DisplayStoredImage => {
                let id = match message.control_data.image_id {
                    Some(id) => id,
                    None => return Err(InvalidControlData::IdMissing.into()),
                };

                Ok(KittyAction::DisplayStoredImage(DisplayStoredImage {
                    image_id: id,
                    placement_id: message
                        .control_data
                        .placement_id
                        .unwrap_or(rand::thread_rng().gen()),
                    placement_data: KittyPlacementData {
                        z_index: message.control_data.z_index,
                        cols: message.control_data.cols,
                        rows: message.control_data.rows,
                        cursor_movement_policy: message.control_data.cursor_movement_policy,
                        unicode_placeholder: message.control_data.unicode_placeholder,
                    },
                }))
            }
            KittyPlacementAction::QuerySupport => {
                let mut action = QuerySupport {
                    image_id: message
                        .control_data
                        .image_id
                        .unwrap_or(rand::thread_rng().gen()),
                    image: KittyImage::try_from(message)?,
                };

                if action.image.metadata.pixel_data_format == KittyPixelDataFormat::Png {
                    action.image = set_kitty_png_size(action.image)?;
                } else {
                    action.image = set_kitty_rgb_headers(action.image)?;
                }

                Ok(KittyAction::QuerySupport(action))
            }
            KittyPlacementAction::Delete => {
                let control_data = &message.control_data;
                // `x=`/`y=` carry the cell coordinates for the positional
                // specifiers and the id bounds for `d=r`. A missing key reads as
                // 0, which is not a valid 1-based cell coordinate or image id,
                // so such a message deletes nothing.
                let (x, y) = (
                    control_data.delete_x.unwrap_or(0),
                    control_data.delete_y.unwrap_or(0),
                );

                let deletion_type = match control_data.delete_action {
                    DeleteAction::All => DeletionType::All,
                    DeleteAction::ById => DeletionType::ById {
                        image_id: match control_data.image_id {
                            Some(image_id) => image_id,
                            None => return Err(InvalidControlData::IdMissing.into()),
                        },
                        placement_id: control_data.placement_id,
                    },
                    DeleteAction::ByNumber => DeletionType::ByNumber {
                        image_number: match control_data.image_number {
                            Some(image_number) => image_number,
                            None => return Err(InvalidControlData::IdMissing.into()),
                        },
                        placement_id: control_data.placement_id,
                    },
                    DeleteAction::AtCursor => DeletionType::AtCursor,
                    DeleteAction::AtPoint => DeletionType::AtPoint { col: x, row: y },
                    DeleteAction::AtPointZ => DeletionType::AtPointZ {
                        col: x,
                        row: y,
                        z: control_data.z_index,
                    },
                    DeleteAction::Column => DeletionType::Column(x),
                    DeleteAction::Row => DeletionType::Row(y),
                    DeleteAction::ZIndex => DeletionType::ZIndex(control_data.z_index),
                    DeleteAction::IdRange => DeletionType::IdRange { start: x, end: y },
                    DeleteAction::Frames => DeletionType::Frames {
                        image_id: match control_data.image_id {
                            Some(image_id) => image_id,
                            None => return Err(InvalidControlData::IdMissing.into()),
                        },
                    },
                    DeleteAction::Unknown => {
                        return Err(InvalidControlData::UnknownDeleteAction.into())
                    }
                };

                Ok(KittyAction::Delete {
                    deletion_type,
                    delete_placements_only: message.control_data.delete_placements_only,
                })
            }
            KittyPlacementAction::Unknown => Err(InvalidKittyAction::UnsupportedAction.into()),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct KittyImageMetadata {
    pub pixel_data_format: KittyPixelDataFormat,
    pub transmission_medium: KittyTransmissionMedium,
    pub image_size: Vector2F,
    /// The `I=` number the client transmitted this image under, if any. Delete
    /// messages using `d=n` select an image by this number instead of by id.
    pub image_number: Option<u32>,
    /// Placements created with `U=1`, keyed by placement id. These have no
    /// anchor in the grid: the cells that reference them carry the row/column
    /// to sample, so the renderer resolves their geometry per frame. Dropping
    /// this image's metadata drops its virtual placements with it, which is why
    /// no other copy of this state is kept.
    pub virtual_placements: HashMap<u32, VirtualPlacement>,
}

/// A `U=1` placement. `rows`/`cols` stay unresolved so that a font-size change
/// re-tiles the image against the new cell size instead of stretching it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VirtualPlacement {
    pub rows: Option<u32>,
    pub cols: Option<u32>,
    pub z_index: i32,
}

#[derive(Debug, Default, Clone)]
pub struct KittyPlacementData {
    pub z_index: i32,
    pub cols: Option<u32>,
    pub rows: Option<u32>,
    pub cursor_movement_policy: CursorMovementPolicy,
    /// Set by `U=1`: the image is not anchored at the cursor, it is drawn
    /// wherever unicode placeholder cells reference it.
    pub unicode_placeholder: bool,
}

impl KittyPlacementData {
    /// The renderer-facing view of a `U=1` placement.
    pub fn virtual_placement(&self) -> VirtualPlacement {
        VirtualPlacement {
            rows: self.rows,
            cols: self.cols,
            z_index: self.z_index,
        }
    }

    pub fn get_desired_dimensions(
        &self,
        image_size: Vector2F,
        cell_height: usize,
        cell_width: usize,
        max_width_px: usize,
        max_height_px: usize,
    ) -> Vector2F {
        let aspect_ratio = image_size.x() / image_size.y();

        let (width_px, height_px) = resize_dimensions(
            image_size.x() as u32,
            image_size.y() as u32,
            min(image_size.x() as u32, max_width_px as u32),
            min(image_size.y() as u32, max_height_px as u32),
            FitType::Contain,
        );

        let (desired_width_px, desired_height_px) = match (self.cols, self.rows) {
            (Some(width), None) => {
                let desired_width_px = ((cell_width as u32) * width) as f32;
                (desired_width_px, desired_width_px / aspect_ratio)
            }
            (None, Some(height)) => {
                let desired_height_px = ((cell_height as u32) * height) as f32;
                (desired_height_px * aspect_ratio, desired_height_px)
            }
            (Some(width), Some(height)) => (
                ((cell_width as u32) * width) as f32,
                ((cell_height as u32) * height) as f32,
            ),
            (None, None) => (width_px as f32, height_px as f32),
        };

        Vector2F::new(desired_width_px, desired_height_px)
    }
}

impl From<KittyControlData> for KittyImageMetadata {
    fn from(control_data: KittyControlData) -> Self {
        Self {
            pixel_data_format: control_data.pixel_data_format,
            image_size: Vector2F::new(control_data.width as f32, control_data.height as f32),
            transmission_medium: control_data.transmission_medium,
            image_number: control_data.image_number,
            virtual_placements: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct KittyChunk {
    pub control_data: KittyControlData,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PendingKittyMessage {
    pub control_data: KittyControlData,
    pub payload: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct KittyMessage {
    pub control_data: KittyControlData,
    pub payload: Vec<u8>,
}

#[derive(Default, Debug, Clone)]
pub enum KittyPlacementAction {
    #[default]
    StoreOnly,
    StoreAndDisplay,
    DisplayStoredImage,
    QuerySupport,
    Delete,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct KittyControlData {
    pixel_data_format: KittyPixelDataFormat,
    width: u32,
    height: u32,
    pub compressed: bool,
    transmission_medium: KittyTransmissionMedium,
    pub further_chunks: bool,
    pub image_id: Option<u32>,
    /// The client assigned image number (`I=`), which delete messages can use in
    /// place of an image id.
    pub image_number: Option<u32>,
    pub placement_id: Option<u32>,
    pub placement_action: KittyPlacementAction,
    pub verbosity: KittyResponseVerbosity,
    pub z_index: i32,
    pub delete_action: DeleteAction,
    /// The `x=` key. It is a crop offset on transmission and either a cell column
    /// or an image id bound on deletion; only the delete reading is implemented.
    pub delete_x: Option<u32>,
    /// The `y=` key. See [`KittyControlData::delete_x`].
    pub delete_y: Option<u32>,
    delete_placements_only: bool,
    pub rows: Option<u32>,
    pub cols: Option<u32>,
    pub cursor_movement_policy: CursorMovementPolicy,
    pub unicode_placeholder: bool,
}

impl Default for KittyControlData {
    fn default() -> Self {
        Self {
            pixel_data_format: KittyPixelDataFormat::default(),
            width: 0,
            height: 0,
            compressed: false,
            transmission_medium: KittyTransmissionMedium::default(),
            further_chunks: false,
            image_id: None,
            image_number: None,
            placement_id: None,
            placement_action: KittyPlacementAction::default(),
            verbosity: KittyResponseVerbosity::default(),
            z_index: 0,
            delete_action: DeleteAction::default(),
            delete_x: None,
            delete_y: None,
            delete_placements_only: true,
            rows: None,
            cols: None,
            cursor_movement_policy: CursorMovementPolicy::default(),
            unicode_placeholder: false,
        }
    }
}

/// The `d=` key of a delete message. The kitty protocol defaults it to `a`.
#[derive(Default, Debug, PartialEq, Clone, Copy)]
pub enum DeleteAction {
    #[default]
    All,
    ById,
    ByNumber,
    AtCursor,
    AtPoint,
    AtPointZ,
    Column,
    Row,
    ZIndex,
    IdRange,
    Frames,
    /// A `d=` letter this terminal does not recognize. Kept distinct from
    /// [`DeleteAction::All`] so that a typo cannot wipe out every image.
    Unknown,
}

#[derive(Default, Debug, PartialEq, Clone, Copy)]
pub enum KittyPixelDataFormat {
    Rgb24Bit,
    #[default]
    Rgba32Bit,
    Png,
}

#[derive(Default, Debug, PartialEq, Clone, Copy)]
pub enum KittyTransmissionMedium {
    #[default]
    Direct,
    SimpleFile,
    TemporaryFile,
    SharedMemoryObject,
}

#[derive(Default, Debug, Clone, Copy)]
pub enum KittyResponseVerbosity {
    #[default]
    All,
    ErrorsOnly,
    None,
}

impl KittyResponseVerbosity {
    pub fn send_ok(&self) -> bool {
        matches!(self, KittyResponseVerbosity::All)
    }

    pub fn send_error(&self) -> bool {
        matches!(self, KittyResponseVerbosity::All)
            || matches!(self, KittyResponseVerbosity::ErrorsOnly)
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub enum CursorMovementPolicy {
    #[default]
    MoveCursor,
    DoNotMoveCursor,
}

fn parse_kitty_control_data(control_data: &[u8]) -> KittyControlData {
    let params: Vec<&[u8]> = control_data.split(|&byte| byte == b',').collect();

    let mut parsed_control_data = KittyControlData::default();
    for param in params {
        let (key, value) = match param.iter().position(|&byte| byte == b'=') {
            Some(position) => (&param[..position], &param[position + 1..]),
            None => continue,
        };

        match key {
            b"f" => {
                parsed_control_data.pixel_data_format = match value {
                    b"24" => KittyPixelDataFormat::Rgb24Bit,
                    b"32" => KittyPixelDataFormat::Rgba32Bit,
                    b"100" => KittyPixelDataFormat::Png,
                    _ => KittyPixelDataFormat::default(),
                };
            }
            b"s" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.width = value;
                }
            }
            b"v" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.height = value;
                }
            }
            b"o" => {
                parsed_control_data.compressed = value == b"z";
            }
            b"m" => {
                parsed_control_data.further_chunks = value == b"1";
            }
            b"t" => {
                parsed_control_data.transmission_medium = match value {
                    b"d" => KittyTransmissionMedium::Direct,
                    b"f" => KittyTransmissionMedium::SimpleFile,
                    b"t" => KittyTransmissionMedium::TemporaryFile,
                    b"s" => KittyTransmissionMedium::SharedMemoryObject,
                    _ => KittyTransmissionMedium::default(),
                }
            }
            b"i" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.image_id = Some(value);
                }
            }
            b"I" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.image_number = Some(value);
                }
            }
            b"p" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.placement_id = Some(value);
                }
            }
            b"a" => {
                parsed_control_data.placement_action = match value {
                    b"t" => KittyPlacementAction::StoreOnly,
                    b"T" => KittyPlacementAction::StoreAndDisplay,
                    b"p" => KittyPlacementAction::DisplayStoredImage,
                    b"q" => KittyPlacementAction::QuerySupport,
                    b"d" => KittyPlacementAction::Delete,
                    _ => KittyPlacementAction::Unknown,
                }
            }
            b"q" => {
                parsed_control_data.verbosity = match value {
                    b"0" => KittyResponseVerbosity::All,
                    b"1" => KittyResponseVerbosity::ErrorsOnly,
                    b"2" => KittyResponseVerbosity::None,
                    _ => KittyResponseVerbosity::default(),
                }
            }
            b"z" => {
                if let Some(value) = parse_i32(value) {
                    parsed_control_data.z_index = value;
                }
            }
            b"d" => {
                // An uppercase specifier additionally frees the stored image data,
                // not just the placements.
                if value.iter().all(|x| x.is_ascii_uppercase()) {
                    parsed_control_data.delete_placements_only = false;
                }

                parsed_control_data.delete_action = match &value.to_ascii_lowercase()[..] {
                    b"a" => DeleteAction::All,
                    b"i" => DeleteAction::ById,
                    b"n" => DeleteAction::ByNumber,
                    b"c" => DeleteAction::AtCursor,
                    b"p" => DeleteAction::AtPoint,
                    b"q" => DeleteAction::AtPointZ,
                    b"x" => DeleteAction::Column,
                    b"y" => DeleteAction::Row,
                    b"z" => DeleteAction::ZIndex,
                    b"r" => DeleteAction::IdRange,
                    b"f" => DeleteAction::Frames,
                    // Deliberately not `DeleteAction::default()`: falling back to
                    // "delete everything" would let one unrecognized letter erase
                    // images the client never asked to remove.
                    _ => DeleteAction::Unknown,
                }
            }
            b"c" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.cols = Some(value);
                }
            }
            b"r" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.rows = Some(value);
                }
            }
            b"C" => {
                parsed_control_data.cursor_movement_policy = match value {
                    b"0" => CursorMovementPolicy::MoveCursor,
                    b"1" => CursorMovementPolicy::DoNotMoveCursor,
                    _ => CursorMovementPolicy::default(),
                }
            }
            b"x" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.delete_x = Some(value);
                }
            }
            b"y" => {
                if let Some(value) = parse_u32(value) {
                    parsed_control_data.delete_y = Some(value);
                }
            }
            b"U" => {
                parsed_control_data.unicode_placeholder = value == b"1";
            }
            _ => {}
        }
    }

    parsed_control_data
}

pub fn parse_kitty_chunk(chunk: Vec<u8>) -> KittyChunk {
    let (control_data, payload) = match chunk.iter().position(|&byte| byte == b';') {
        Some(position) => (&chunk[..position], &chunk[position + 1..]),
        None => (&chunk[..], &vec![][..]),
    };

    let control_data = parse_kitty_control_data(control_data);

    KittyChunk {
        control_data,
        payload: payload.to_vec(),
    }
}

#[cfg(feature = "local_fs")]
fn read_file(decoded_payload: Vec<u8>, is_temp: bool) -> Result<Vec<u8>, InvalidKittyPayload> {
    let path = match str::from_utf8(&decoded_payload[..]) {
        Ok(path) => path,
        Err(err) => return Err(KittyDecodeError::InvalidUtf8(err.to_string()).into()),
    };

    let data = match fs::read(path) {
        Ok(data) => data,
        Err(err) => {
            return Err(InvalidKittyPayload::FileError(FileError::FileReadError(
                err.to_string(),
            )))
        }
    };

    if is_temp {
        safe_delete_temp_file(path);
    }

    Ok(data)
}

#[cfg(feature = "local_fs")]
fn safe_delete_temp_file(path: &str) {
    if is_path_in_temp_dir(path) && path.contains("tty-graphics-protocol") {
        if let Err(err) = fs::remove_file(path) {
            log::error!("Failed to delete kitty temporary file (path = {path}): {err}");
        }
    }
}

#[cfg(feature = "local_fs")]
fn is_path_in_temp_dir(path: &str) -> bool {
    let temp_dirs = vec!["/tmp", "/var/tmp", "/dev/shm"];

    for temp_dir in temp_dirs {
        if path.starts_with(temp_dir) {
            return true;
        }
    }

    if let Ok(temp_dir) = env::var("TMPDIR") {
        if path.starts_with(&temp_dir) {
            return true;
        }
    }

    false
}

#[cfg(all(feature = "local_fs", unix))]
fn read_shared_memory(
    control_data: KittyControlData,
    decoded_payload: Vec<u8>,
) -> Result<Vec<u8>, InvalidKittyPayload> {
    use nix::sys::mman::{shm_open, shm_unlink};

    let path = match str::from_utf8(&decoded_payload[..]) {
        Ok(path) => path,
        Err(err) => return Err(KittyDecodeError::InvalidUtf8(err.to_string()).into()),
    };

    let fd = match shm_open(
        path,
        nix::fcntl::OFlag::O_RDONLY,
        nix::sys::stat::Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(err) => {
            return Err(InvalidKittyPayload::ShmError(ShmError::ObjectOpenError(
                err.to_string(),
            )))
        }
    };
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };

    let bytes_per_pixel = match control_data.pixel_data_format {
        KittyPixelDataFormat::Rgb24Bit => Some(3),
        KittyPixelDataFormat::Rgba32Bit => Some(4),
        KittyPixelDataFormat::Png => None,
    };

    let size = bytes_per_pixel.map(|bytes_per_pixel| {
        (bytes_per_pixel * control_data.width * control_data.height) as usize
    });

    let data = read_from_shared_memory_fd(&fd, size);

    if let Err(err) = shm_unlink(path) {
        log::warn!("Failed to unlink kitty shm file (path = {path}): {err:?}");
    };

    data
}

#[cfg(all(feature = "local_fs", unix))]
struct MappedRegion {
    pointer: *mut libc::c_void,
    len: usize,
}

#[cfg(all(feature = "local_fs", unix))]
impl MappedRegion {
    unsafe fn as_slice(&self) -> &[u8] {
        std::slice::from_raw_parts(self.pointer.cast(), self.len)
    }
}

#[cfg(all(feature = "local_fs", unix))]
impl Drop for MappedRegion {
    fn drop(&mut self) {
        if let Err(error) = unsafe { nix::sys::mman::munmap(self.pointer, self.len) } {
            log::error!("Failed to unmap kitty shared memory: {error}");
        }
    }
}

#[cfg(all(feature = "local_fs", unix))]
fn read_from_shared_memory_fd(
    fd: &OwnedFd,
    size: Option<usize>,
) -> Result<Vec<u8>, InvalidKittyPayload> {
    use nix::sys::{
        mman::{mmap, MapFlags, ProtFlags},
        stat::fstat,
    };
    use std::num::NonZero;

    let file_size = match fstat(fd.as_raw_fd()) {
        Ok(stat) => stat.st_size,
        Err(err) => {
            return Err(InvalidKittyPayload::ShmError(ShmError::FileStatError(
                err.to_string(),
            )))
        }
    };

    let Ok(file_size) = usize::try_from(file_size) else {
        return Err(InvalidKittyPayload::ShmError(ShmError::InvalidObjectSize));
    };

    let size = size.unwrap_or(file_size);

    if file_size < size {
        return Err(InvalidKittyPayload::ShmError(ShmError::ObjectTooSmall {
            expected_bytes: size,
            actual_bytes: file_size,
        }));
    }

    let Some(size) = NonZero::new(size) else {
        return Err(InvalidKittyPayload::ShmError(ShmError::EmptyObject));
    };

    let ptr = unsafe {
        mmap(
            None,
            size,
            ProtFlags::PROT_READ,
            MapFlags::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };

    let Ok(ptr) = ptr else {
        return Err(InvalidKittyPayload::ShmError(ShmError::MmapError));
    };

    let region = MappedRegion {
        pointer: ptr,
        len: size.into(),
    };
    let data = unsafe { region.as_slice() }.to_vec();

    Ok(data)
}

fn base64_decode_padding_agnostic(data: &[u8]) -> Result<Vec<u8>, KittyDecodeError> {
    if let Ok(decoded_bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
        return Ok(decoded_bytes);
    }

    match base64::engine::general_purpose::STANDARD_NO_PAD.decode(data) {
        Ok(decoded_bytes) => Ok(decoded_bytes),
        Err(err) => Err(KittyDecodeError::InvalidBase64(err.to_string())),
    }
}

#[cfg(test)]
#[path = "kitty_tests.rs"]
mod tests;

pub fn decode_kitty_image_data(
    payload: Vec<u8>,
    compressed: bool,
) -> Result<Vec<u8>, KittyDecodeError> {
    let mut result = payload;

    if compressed {
        let mut decoder = ZlibDecoder::new(&result[..]);
        let mut decompresed_bytes = vec![];
        match decoder.read_to_end(&mut decompresed_bytes) {
            Ok(_) => {}
            Err(err) => return Err(KittyDecodeError::InvalidCompression(err.to_string())),
        };
        result = decompresed_bytes;
    }

    Ok(result)
}

pub fn set_kitty_png_size(mut image: KittyImage) -> Result<KittyImage, KittyPngError> {
    let image_type = match ImageType::try_from_bytes(&image.data[..]) {
        Ok(image_type) => image_type,
        Err(err) => return Err(KittyPngError::InvalidBytes(err.to_string())),
    };
    let image_size = match image_type.image_size() {
        Some(image_size) => image_size,
        None => {
            return Err(KittyPngError::InvalidBytes(
                "Could not retrieve image size from ImageType for Kitty PNG.".to_string(),
            ))
        }
    };

    image.metadata.image_size = image_size.to_f32();

    Ok(image)
}

pub fn set_kitty_rgb_headers(mut image: KittyImage) -> Result<KittyImage, KittyRgbError> {
    let image_format = match image.metadata.pixel_data_format {
        KittyPixelDataFormat::Rgb24Bit => CustomImageFormat::Rgb,
        KittyPixelDataFormat::Rgba32Bit => CustomImageFormat::Rgba,
        KittyPixelDataFormat::Png => return Ok(image),
    };

    image.data = CustomImageHeader::prepend_custom_header(
        image.data,
        image.metadata.image_size.x() as u32,
        image.metadata.image_size.y() as u32,
        image_format,
    )?;

    Ok(image)
}

/// The error code prefix the kitty graphics protocol expects a failed response
/// to start with, including its trailing colon.
fn kitty_error_code(err: &KittyError) -> String {
    let code = match err {
        KittyError::KittyFeatureDisabled => "ENOTSUPP",
        KittyError::StorageError(StorageError::UnknownId { .. }) => "ENOENT",
        KittyError::InvalidKittyAction(action) => match action {
            InvalidKittyAction::UnsupportedAction => "ENOTSUPP",
            InvalidKittyAction::InvalidControlData(_) => "EINVAL",
            InvalidKittyAction::InvalidKittyPayload(payload) => match payload {
                InvalidKittyPayload::InvalidTransmissionMedium(_)
                | InvalidKittyPayload::KittyDecodeError(_)
                | InvalidKittyPayload::InvalidKittyImage(_) => "EINVAL",
                InvalidKittyPayload::FileError(FileError::UnsupportedPlatform)
                | InvalidKittyPayload::ShmError(ShmError::UnsupportedPlatform) => "ENOTSUPP",
                InvalidKittyPayload::FileError(_) | InvalidKittyPayload::ShmError(_) => "EBADF",
            },
        },
    };

    format!("{code}:")
}

fn create_kitty_reply(image_id: u32, placement_id: Option<u32>, message: String) -> Vec<u8> {
    // 0 is not a valid placement id, so a client that left `p` at its zero
    // default gets it omitted from the reply, matching kitty. Exact-match ack
    // parsers rely on this.
    let identifiers = match placement_id.filter(|&id| id != 0) {
        Some(placement_id) => format!("i={image_id},p={placement_id}"),
        None => format!("i={image_id}"),
    };

    [
        C1::APC,
        b"G",
        format!("{identifiers};{message}").as_bytes(),
        C1::ST,
    ]
    .concat()
}

pub fn create_kitty_ok_reply(image_id: u32, placement_id: Option<u32>) -> Vec<u8> {
    create_kitty_reply(image_id, placement_id, "OK".to_string())
}

pub fn create_kitty_error_reply(
    image_id: u32,
    placement_id: Option<u32>,
    err: KittyError,
) -> Vec<u8> {
    let message = format!("{}{err:?}", kitty_error_code(&err));
    create_kitty_reply(image_id, placement_id, message)
}
