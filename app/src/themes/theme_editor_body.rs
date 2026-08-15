use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use pathfinder_color::ColorU;
use warp_core::ui::theme::{Fill as ThemeFill, HorizontalGradient, VerticalGradient, WarpTheme};
use warpui::elements::{
    Border, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Fill, Flex, MainAxisSize, MouseStateHandle, Padding,
    ParentElement, Radius, Rect, ScrollbarWidth, Shrinkable, Text,
};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::text_input::TextInput;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::appearance::{Appearance, AppearanceManager};
use crate::editor::{EditorView, Event as EditorEvent};
use crate::settings::Settings;
use crate::themes::default_themes::{light_theme, vscode_2026_dark, zaplex_dark};
use crate::themes::theme::{CustomTheme, InMemoryThemeOptions, ThemeKind};
use crate::themes::theme_creator::{format_theme_color, parse_theme_color_input};
#[cfg(feature = "local_fs")]
use crate::themes::theme_creator_body::ThemeCreatorBody;
#[cfg(feature = "local_fs")]
use crate::user_config;

const CONTROL_RADIUS: f32 = 6.0;
const CARD_RADIUS: f32 = 10.0;
const EDITOR_WIDTH: f32 = 94.0;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TemplateChoice {
    Current,
    ZaplexDark,
    VsCode2026Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ThemeFillField {
    Background,
    Accent,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum FillMode {
    Solid,
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ThemeColorField {
    BackgroundStart,
    BackgroundEnd,
    Foreground,
    AccentStart,
    AccentEnd,
    Outline,
    Selection,
    Cursor,
    AnsiNormal(&'static str),
    AnsiBright(&'static str),
    Ui(&'static str),
}

impl ThemeColorField {
    fn key(self) -> String {
        match self {
            Self::BackgroundStart => "background.start".into(),
            Self::BackgroundEnd => "background.end".into(),
            Self::Foreground => "foreground".into(),
            Self::AccentStart => "accent.start".into(),
            Self::AccentEnd => "accent.end".into(),
            Self::Outline => "outline".into(),
            Self::Selection => "selection".into(),
            Self::Cursor => "cursor".into(),
            Self::AnsiNormal(name) => format!("ansi.normal.{name}"),
            Self::AnsiBright(name) => format!("ansi.bright.{name}"),
            Self::Ui(name) => format!("ui.{name}"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum PendingAction {
    Close,
    Templates,
    Import,
    Image,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum GuardDecision {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Debug)]
pub(crate) enum ThemeEditorBodyAction {
    ChooseTemplate(TemplateChoice),
    RequestClose,
    RequestTemplates,
    RequestImport,
    RequestImage,
    Export,
    Save,
    DiscardDraft,
    ResolveGuard(GuardDecision),
    SetFillMode(ThemeFillField, FillMode),
    ToggleUiColors,
    ResetUiColors,
    HandleImageSelected(PathBuf),
    HandleImportSelected(PathBuf),
    FilePickerCancelled,
}

pub enum ThemeEditorBodyEvent {
    Close,
    OpenImagePicker,
    OpenImportPicker,
    ExportYaml { filename: String, yaml: String },
    SetCustomTheme { theme: ThemeKind },
    ShowErrorToast { message: String },
}

struct ThemeDraft {
    theme: WarpTheme,
    name: String,
    dirty: bool,
    image_source: Option<PathBuf>,
}

#[derive(Default)]
struct ButtonStates {
    templates: Vec<MouseStateHandle>,
    image: MouseStateHandle,
    import: MouseStateHandle,
    export: MouseStateHandle,
    close: MouseStateHandle,
    save: MouseStateHandle,
    discard: MouseStateHandle,
    back: MouseStateHandle,
    ui_toggle: MouseStateHandle,
    ui_reset: MouseStateHandle,
    fill_modes: Vec<MouseStateHandle>,
    guard_save: MouseStateHandle,
    guard_discard: MouseStateHandle,
    guard_cancel: MouseStateHandle,
}

pub(crate) struct ThemeEditorBody {
    scroll: ClippedScrollStateHandle,
    buttons: ButtonStates,
    name_editor: ViewHandle<EditorView>,
    color_editors: HashMap<String, ViewHandle<EditorView>>,
    color_values: HashMap<String, String>,
    color_fields: Vec<ThemeColorField>,
    invalid_fields: HashSet<String>,
    draft: Option<ThemeDraft>,
    ui_colors_expanded: bool,
    pending_action: Option<PendingAction>,
    image_loading: bool,
}

impl ThemeEditorBody {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let name_editor = ctx.add_typed_action_view(|ctx| EditorView::new(Default::default(), ctx));
        ctx.subscribe_to_view(&name_editor, |me, editor, event, ctx| {
            if matches!(event, EditorEvent::Edited(_)) {
                if let Some(draft) = &mut me.draft {
                    draft.name = editor.as_ref(ctx).buffer_text(ctx);
                    draft.theme.set_name(draft.name.clone());
                    draft.dirty = true;
                }
                ctx.notify();
            }
        });

        let color_fields = all_color_fields();
        let mut color_editors = HashMap::new();
        for field in &color_fields {
            let key = field.key();
            let editor = ctx.add_typed_action_view(|ctx| EditorView::new(Default::default(), ctx));
            let event_key = key.clone();
            let event_field = *field;
            ctx.subscribe_to_view(&editor, move |me, editor, event, ctx| {
                if matches!(event, EditorEvent::Edited(_)) {
                    let value = editor.as_ref(ctx).buffer_text(ctx);
                    me.color_values.insert(event_key.clone(), value.clone());
                    me.apply_color_input(event_field, &event_key, value, ctx);
                }
            });
            color_editors.insert(key, editor);
        }

        Self {
            scroll: Default::default(),
            buttons: ButtonStates {
                templates: (0..4).map(|_| MouseStateHandle::default()).collect(),
                fill_modes: (0..6).map(|_| MouseStateHandle::default()).collect(),
                ..Default::default()
            },
            name_editor,
            color_editors,
            color_values: HashMap::new(),
            color_fields,
            invalid_fields: HashSet::new(),
            draft: None,
            ui_colors_expanded: false,
            pending_action: None,
            image_loading: false,
        }
    }

    pub fn request_close(&mut self, ctx: &mut ViewContext<Self>) {
        self.request_action(PendingAction::Close, ctx);
    }

    fn request_action(&mut self, action: PendingAction, ctx: &mut ViewContext<Self>) {
        if self.draft.as_ref().is_some_and(|draft| draft.dirty) {
            self.pending_action = Some(action);
            ctx.notify();
        } else {
            self.execute_pending(action, ctx);
        }
    }

    fn execute_pending(&mut self, action: PendingAction, ctx: &mut ViewContext<Self>) {
        match action {
            PendingAction::Close => {
                self.clear_transient(ctx);
                self.draft = None;
                self.invalid_fields.clear();
                self.image_loading = false;
                ctx.emit(ThemeEditorBodyEvent::Close);
            }
            PendingAction::Templates => {
                self.clear_transient(ctx);
                self.draft = None;
                self.invalid_fields.clear();
                ctx.notify();
            }
            PendingAction::Import => {
                ctx.emit(ThemeEditorBodyEvent::OpenImportPicker);
            }
            PendingAction::Image => {
                self.image_loading = true;
                ctx.emit(ThemeEditorBodyEvent::OpenImagePicker);
                ctx.notify();
            }
        }
    }

    fn resolve_guard(&mut self, decision: GuardDecision, ctx: &mut ViewContext<Self>) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        match decision {
            GuardDecision::Save => {
                if self.persist_draft(ctx) {
                    self.execute_pending(action, ctx);
                } else {
                    self.pending_action = Some(action);
                }
            }
            GuardDecision::Discard => {
                if matches!(action, PendingAction::Import | PendingAction::Image) {
                    self.clear_transient(ctx);
                    self.draft = None;
                    self.invalid_fields.clear();
                }
                self.execute_pending(action, ctx);
            }
            GuardDecision::Cancel => ctx.notify(),
        }
    }

    fn choose_template(&mut self, choice: TemplateChoice, ctx: &mut ViewContext<Self>) {
        let (name, theme) = match choice {
            TemplateChoice::Current => (
                crate::t!("theme-editor-template-current"),
                Appearance::as_ref(ctx).theme().clone(),
            ),
            TemplateChoice::ZaplexDark => (
                ThemeKind::ZaplexDark.to_string(),
                Settings::theme_for_theme_kind(&ThemeKind::ZaplexDark, ctx),
            ),
            TemplateChoice::VsCode2026Dark => (
                ThemeKind::VsCode2026Dark.to_string(),
                Settings::theme_for_theme_kind(&ThemeKind::VsCode2026Dark, ctx),
            ),
            TemplateChoice::Light => (
                ThemeKind::Light.to_string(),
                Settings::theme_for_theme_kind(&ThemeKind::Light, ctx),
            ),
        };
        self.load_theme(theme, format!("{name} Copy"), true, None, ctx);
    }

    fn load_theme(
        &mut self,
        mut theme: WarpTheme,
        name: String,
        dirty: bool,
        image_source: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        theme.set_name(name.clone());
        self.draft = Some(ThemeDraft {
            theme: theme.clone(),
            name: name.clone(),
            dirty,
            image_source,
        });
        self.invalid_fields.clear();
        self.name_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text(&name, ctx));
        self.refresh_color_editors(ctx);
        self.apply_live_theme(theme, ctx);
        ctx.notify();
    }

    fn refresh_color_editors(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        let theme = draft.theme.clone();
        for field in &self.color_fields {
            let key = field.key();
            let value = color_for_field(&theme, *field);
            if let Some(editor) = self.color_editors.get(&key) {
                let formatted = format_theme_color(value);
                self.color_values.insert(key.clone(), formatted.clone());
                editor.update(ctx, |editor, ctx| editor.set_buffer_text(&formatted, ctx));
            }
        }
    }

    fn apply_color_input(
        &mut self,
        field: ThemeColorField,
        key: &str,
        value: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let Ok(color) = parse_theme_color_input(&value) else {
            self.invalid_fields.insert(key.to_owned());
            ctx.notify();
            return;
        };
        let Some(draft) = &self.draft else {
            return;
        };
        let mut theme = draft.theme.clone();
        let update = match field {
            ThemeColorField::BackgroundStart | ThemeColorField::BackgroundEnd => self
                .updated_fill(ThemeFillField::Background, field, color)
                .and_then(|fill| set_theme_fill(&theme, "background", fill)),
            ThemeColorField::AccentStart | ThemeColorField::AccentEnd => self
                .updated_fill(ThemeFillField::Accent, field, color)
                .and_then(|fill| set_theme_fill(&theme, "accent", fill)),
            ThemeColorField::Foreground => set_theme_color(&theme, &["foreground"], color),
            ThemeColorField::Outline => set_theme_color(&theme, &["ui_colors", "border"], color),
            ThemeColorField::Selection => {
                set_theme_color(&theme, &["ui_colors", "selection"], color)
            }
            ThemeColorField::Cursor => set_theme_color(&theme, &["cursor"], color),
            ThemeColorField::AnsiNormal(name) => {
                set_theme_color(&theme, &["terminal_colors", "normal", name], color)
            }
            ThemeColorField::AnsiBright(name) => {
                set_theme_color(&theme, &["terminal_colors", "bright", name], color)
            }
            ThemeColorField::Ui(name) => set_theme_color(&theme, &["ui_colors", name], color),
        };
        match update {
            Ok(updated) => {
                theme = updated;
                self.invalid_fields.remove(key);
                if let Some(draft) = &mut self.draft {
                    draft.theme = theme.clone();
                    draft.dirty = true;
                }
                self.apply_live_theme(theme, ctx);
            }
            Err(error) => {
                self.invalid_fields.insert(key.to_owned());
                log::warn!("Theme editor rejected {key}: {error}");
            }
        }
        ctx.notify();
    }

    fn updated_fill(
        &self,
        fill_field: ThemeFillField,
        edited_field: ThemeColorField,
        edited_color: ColorU,
    ) -> anyhow::Result<ThemeFill> {
        let (start_key, end_key) = match fill_field {
            ThemeFillField::Background => ("background.start", "background.end"),
            ThemeFillField::Accent => ("accent.start", "accent.end"),
        };
        let start = if matches!(
            edited_field,
            ThemeColorField::BackgroundStart | ThemeColorField::AccentStart
        ) {
            edited_color
        } else {
            self.editor_color(start_key)?
        };
        let end = if matches!(
            edited_field,
            ThemeColorField::BackgroundEnd | ThemeColorField::AccentEnd
        ) {
            edited_color
        } else {
            self.editor_color(end_key)?
        };
        Ok(match self.fill_mode(fill_field) {
            FillMode::Solid => ThemeFill::Solid(start),
            FillMode::Vertical => ThemeFill::VerticalGradient(VerticalGradient::new(start, end)),
            FillMode::Horizontal => {
                ThemeFill::HorizontalGradient(HorizontalGradient::new(start, end))
            }
        })
    }

    fn editor_color(&self, key: &str) -> anyhow::Result<ColorU> {
        let value = self
            .color_values
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("missing color value {key}"))?;
        parse_theme_color_input(value)
    }

    fn fill_mode(&self, field: ThemeFillField) -> FillMode {
        let Some(draft) = &self.draft else {
            return FillMode::Solid;
        };
        let fill = match field {
            ThemeFillField::Background => draft.theme.background(),
            ThemeFillField::Accent => draft.theme.accent(),
        };
        match fill {
            ThemeFill::Solid(_) => FillMode::Solid,
            ThemeFill::VerticalGradient(_) => FillMode::Vertical,
            ThemeFill::HorizontalGradient(_) => FillMode::Horizontal,
        }
    }

    fn set_fill_mode(
        &mut self,
        field: ThemeFillField,
        mode: FillMode,
        ctx: &mut ViewContext<Self>,
    ) {
        let (start_key, end_key, yaml_key) = match field {
            ThemeFillField::Background => ("background.start", "background.end", "background"),
            ThemeFillField::Accent => ("accent.start", "accent.end", "accent"),
        };
        let Ok(start) = self.editor_color(start_key) else {
            self.invalid_fields.insert(start_key.into());
            ctx.notify();
            return;
        };
        let end = self.editor_color(end_key).unwrap_or(start);
        let fill = match mode {
            FillMode::Solid => ThemeFill::Solid(start),
            FillMode::Vertical => ThemeFill::VerticalGradient(VerticalGradient::new(start, end)),
            FillMode::Horizontal => {
                ThemeFill::HorizontalGradient(HorizontalGradient::new(start, end))
            }
        };
        let Some(draft) = &self.draft else {
            return;
        };
        match set_theme_fill(&draft.theme, yaml_key, fill) {
            Ok(theme) => {
                if let Some(draft) = &mut self.draft {
                    draft.theme = theme.clone();
                    draft.dirty = true;
                }
                self.apply_live_theme(theme, ctx);
                ctx.notify();
            }
            Err(error) => self.send_error(
                crate::t!("theme-editor-error-gradient", error = error.to_string()),
                ctx,
            ),
        }
    }

    fn reset_ui_colors(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        match reset_ui_color_overrides(&draft.theme) {
            Ok(theme) => {
                if let Some(draft) = &mut self.draft {
                    draft.theme = theme.clone();
                    draft.dirty = true;
                }
                self.refresh_color_editors(ctx);
                self.apply_live_theme(theme, ctx);
            }
            Err(error) => self.send_error(
                crate::t!("theme-editor-error-ui-reset", error = error.to_string()),
                ctx,
            ),
        }
    }

    fn apply_live_theme(&self, theme: WarpTheme, ctx: &mut ViewContext<Self>) {
        AppearanceManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.set_transient_warp_theme(theme, ctx)
        });
    }

    pub(super) fn clear_transient(&self, ctx: &mut ViewContext<Self>) {
        AppearanceManager::handle(ctx)
            .update(ctx, |manager, ctx| manager.clear_transient_theme(ctx));
    }

    fn persist_draft(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if !self.invalid_fields.is_empty() {
            self.send_error(crate::t!("theme-editor-invalid-save"), ctx);
            return false;
        }
        let Some(draft) = self.draft.as_ref() else {
            return false;
        };
        let filename = safe_theme_filename(&draft.name);
        if filename.is_empty() {
            self.send_error(crate::t!("theme-editor-invalid-name"), ctx);
            return false;
        }
        let name = draft.name.clone();
        let mut theme = draft.theme.clone();
        theme.set_name(name.clone());
        let image_source = draft.image_source.clone();

        #[cfg(feature = "local_fs")]
        let (saved, saved_theme) = {
            let themes_dir = user_config::themes_dir();
            let mut saved_theme = theme;
            let image_copy = if let Some(source) = image_source {
                let Some(extension) = source
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_owned)
                else {
                    self.send_error(crate::t!("theme-creator-process-image-failed"), ctx);
                    return false;
                };
                let destination = themes_dir.join(format!("{filename}.{extension}"));
                saved_theme = match set_theme_image_path(&saved_theme, &destination) {
                    Ok(theme) => theme,
                    Err(error) => {
                        self.send_error(
                            crate::t!(
                                "theme-editor-error-image-prepare",
                                error = error.to_string()
                            ),
                            ctx,
                        );
                        return false;
                    }
                };
                (source != destination).then_some((source, filename.clone(), extension))
            } else {
                None
            };
            let saved = match image_copy {
                Some((source, image_name, extension)) => ThemeCreatorBody::write_theme(
                    &saved_theme,
                    themes_dir,
                    format!("{filename}.yaml"),
                    Some((source, image_name, &extension)),
                    |path| path,
                ),
                None => ThemeCreatorBody::write_theme(
                    &saved_theme,
                    themes_dir,
                    format!("{filename}.yaml"),
                    None,
                    |path| path,
                ),
            };
            (saved, saved_theme)
        };
        #[cfg(not(feature = "local_fs"))]
        let (saved, saved_theme): (Option<PathBuf>, WarpTheme) = (None, theme);
        let Some(path) = saved else {
            self.send_error(crate::t!("common-something-went-wrong"), ctx);
            return false;
        };
        if let Some(draft) = &mut self.draft {
            draft.theme = saved_theme;
            draft.dirty = false;
            draft.image_source = None;
        }
        ctx.emit(ThemeEditorBodyEvent::SetCustomTheme {
            theme: ThemeKind::Custom(CustomTheme::new(name, path)),
        });
        ctx.notify();
        true
    }

    fn export(&self, ctx: &mut ViewContext<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        if !self.invalid_fields.is_empty() {
            self.send_error(crate::t!("theme-editor-invalid-save"), ctx);
            return;
        }
        let filename = safe_theme_filename(&draft.name);
        if filename.is_empty() {
            self.send_error(crate::t!("theme-editor-invalid-name"), ctx);
            return;
        }
        match serde_yaml::to_string(&draft.theme) {
            Ok(yaml) => ctx.emit(ThemeEditorBodyEvent::ExportYaml {
                filename: format!("{filename}.yaml"),
                yaml,
            }),
            Err(error) => self.send_error(
                crate::t!("theme-editor-error-export", error = error.to_string()),
                ctx,
            ),
        }
    }

    fn import_theme(&mut self, path: &Path, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_fs")]
        match std::fs::read_to_string(path)
            .map_err(anyhow::Error::from)
            .and_then(|yaml| serde_yaml::from_str::<WarpTheme>(&yaml).map_err(anyhow::Error::from))
        {
            Ok(theme) => {
                let name = theme.name().unwrap_or_else(|| {
                    path.file_stem()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Imported Theme".into())
                });
                self.load_theme(theme, name, true, None, ctx);
            }
            Err(error) => self.send_error(
                crate::t!("theme-editor-error-import", error = error.to_string()),
                ctx,
            ),
        }
        #[cfg(not(feature = "local_fs"))]
        self.send_error(crate::t!("theme-editor-error-local-files"), ctx);
    }

    fn image_theme(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let name = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Image Theme".into());
        let image_source = path.clone();
        ctx.spawn(
            InMemoryThemeOptions::new(name.clone(), path),
            move |me, result, ctx| {
                me.image_loading = false;
                match result {
                    Ok(options) => {
                        me.load_theme(options.theme(), name, true, Some(image_source), ctx)
                    }
                    Err(error) => me.send_error(
                        crate::t!("theme-editor-error-image", error = error.to_string()),
                        ctx,
                    ),
                }
                ctx.notify();
            },
        );
    }

    fn send_error(&self, message: String, ctx: &mut ViewContext<Self>) {
        ctx.emit(ThemeEditorBodyEvent::ShowErrorToast { message });
    }

    fn render_button(
        &self,
        label: String,
        state: MouseStateHandle,
        variant: ButtonVariant,
        action: ThemeEditorBodyAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(variant, state)
            .with_style(UiComponentStyles {
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS))),
                padding: Some(Coords::uniform(8.0)),
                ..Default::default()
            })
            .with_centered_text_label(label)
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish()
    }

    fn render_template_selection(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut cards = Flex::row()
            .with_spacing(8.0)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        let choices = [
            (
                TemplateChoice::Current,
                crate::t!("theme-editor-template-current"),
                appearance.theme().clone(),
            ),
            (
                TemplateChoice::ZaplexDark,
                "Zaplex Dark".into(),
                zaplex_dark(),
            ),
            (
                TemplateChoice::VsCode2026Dark,
                "VS Code 2026 Dark".into(),
                vscode_2026_dark(),
            ),
            (TemplateChoice::Light, "Light".into(), light_theme()),
        ];
        for (index, (choice, label, theme)) in choices.into_iter().enumerate() {
            cards = cards.with_child(
                Shrinkable::new(
                    1.0,
                    self.render_template_card(
                        label,
                        &theme,
                        self.buttons.templates[index].clone(),
                        choice,
                        appearance,
                    ),
                )
                .finish(),
            );
        }
        let sources = Flex::row()
            .with_spacing(8.0)
            .with_child(self.render_button(
                crate::t!("theme-editor-import"),
                self.buttons.import.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::RequestImport,
                appearance,
            ))
            .with_child(self.render_button(
                if self.image_loading {
                    crate::t!("theme-creator-selecting-image")
                } else {
                    crate::t!("theme-editor-from-image")
                },
                self.buttons.image.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::RequestImage,
                appearance,
            ))
            .with_child(self.render_button(
                crate::t!("common-close"),
                self.buttons.close.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::RequestClose,
                appearance,
            ))
            .finish();
        Flex::column()
            .with_spacing(16.0)
            .with_child(self.heading(crate::t!("theme-editor-choose-template"), appearance))
            .with_child(cards.finish())
            .with_child(sources)
            .finish()
    }

    fn render_template_card(
        &self,
        label: String,
        theme: &WarpTheme,
        state: MouseStateHandle,
        choice: TemplateChoice,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut palette = Flex::row();
        for color in [
            theme.background().into_solid(),
            theme.surface_2().into_solid(),
            theme.accent().into_solid(),
            theme.foreground().into_solid(),
        ] {
            palette = palette.with_child(
                Shrinkable::new(
                    1.0,
                    ConstrainedBox::new(Rect::new().with_background_color(color).finish())
                        .with_height(44.0)
                        .finish(),
                )
                .finish(),
            );
        }
        Container::new(
            Flex::column()
                .with_spacing(8.0)
                .with_child(
                    Container::new(palette.finish())
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS)))
                        .finish(),
                )
                .with_child(self.render_button(
                    label,
                    state,
                    ButtonVariant::Secondary,
                    ThemeEditorBodyAction::ChooseTemplate(choice),
                    appearance,
                ))
                .finish(),
        )
        .with_background(appearance.theme().surface_2())
        .with_border(Border::all(1.0).with_border_fill(appearance.theme().outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CARD_RADIUS)))
        .with_padding(Padding::uniform(10.0))
        .finish()
    }

    fn render_editor(&self, appearance: &Appearance) -> Box<dyn Element> {
        let draft = self.draft.as_ref().expect("editor requires a draft");
        let toolbar = Flex::row()
            .with_spacing(8.0)
            .with_child(self.render_button(
                crate::t!("theme-editor-templates"),
                self.buttons.back.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::RequestTemplates,
                appearance,
            ))
            .with_child(self.render_button(
                crate::t!("theme-editor-import"),
                self.buttons.import.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::RequestImport,
                appearance,
            ))
            .with_child(self.render_button(
                crate::t!("theme-editor-export"),
                self.buttons.export.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::Export,
                appearance,
            ))
            .with_child(self.render_button(
                if self.image_loading {
                    crate::t!("theme-creator-selecting-image")
                } else {
                    crate::t!("theme-editor-from-image")
                },
                self.buttons.image.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::RequestImage,
                appearance,
            ))
            .with_child(self.render_button(
                crate::t!("common-close"),
                self.buttons.close.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::RequestClose,
                appearance,
            ))
            .finish();

        let name = self.render_text_input(&self.name_editor, false, appearance);
        let mut inspector = Flex::column()
            .with_spacing(16.0)
            .with_child(
                self.card(
                    Flex::column()
                        .with_spacing(8.0)
                        .with_child(self.label(crate::t!("theme-creator-theme-name"), appearance))
                        .with_child(name)
                        .finish(),
                    appearance,
                ),
            )
            .with_child(self.render_primary_colors(appearance))
            .with_child(self.render_ansi_colors(appearance))
            .with_child(self.render_ui_colors(appearance));

        if !self.invalid_fields.is_empty() {
            inspector = inspector.with_child(
                Text::new_inline(
                    crate::t!("theme-editor-invalid-color"),
                    appearance.ui_font_family(),
                    appearance.ui_font_body(),
                )
                .with_color(appearance.theme().ui_error_color())
                .finish(),
            );
        }

        let inspector = ClippedScrollable::vertical(
            self.scroll.clone(),
            Container::new(inspector.finish())
                .with_padding(Padding::uniform(16.0))
                .finish(),
            ScrollbarWidth::Auto,
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            Fill::None,
        )
        .finish();
        let workspace = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                ConstrainedBox::new(
                    Container::new(inspector)
                        .with_background(appearance.theme().surface_1())
                        .with_border(
                            Border::all(1.0).with_border_fill(appearance.theme().outline()),
                        )
                        .finish(),
                )
                .with_width(410.0)
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.0,
                    Container::new(self.render_preview(&draft.theme, appearance))
                        .with_padding(Padding::uniform(16.0))
                        .finish(),
                )
                .finish(),
            )
            .finish();

        let footer = Flex::row()
            .with_spacing(8.0)
            .with_child(self.render_button(
                crate::t!("theme-editor-discard"),
                self.buttons.discard.clone(),
                ButtonVariant::Secondary,
                ThemeEditorBodyAction::DiscardDraft,
                appearance,
            ))
            .with_child(self.render_button(
                crate::t!("theme-editor-save"),
                self.buttons.save.clone(),
                ButtonVariant::Accent,
                ThemeEditorBodyAction::Save,
                appearance,
            ))
            .finish();

        let mut content = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.0)
            .with_child(
                Container::new(toolbar)
                    .with_padding(Padding::uniform(8.0))
                    .finish(),
            );
        if self.pending_action.is_some() {
            content = content.with_child(self.render_guard(appearance));
        }
        content
            .with_child(Shrinkable::new(1.0, workspace).finish())
            .with_child(
                Container::new(footer)
                    .with_padding(Padding::uniform(8.0))
                    .finish(),
            )
            .finish()
    }

    fn render_primary_colors(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut content = Flex::column()
            .with_spacing(8.0)
            .with_child(self.heading(crate::t!("theme-editor-primary-colors"), appearance))
            .with_child(self.render_fill_editor(ThemeFillField::Background, appearance))
            .with_child(self.render_color_field(
                crate::t!("theme-editor-foreground"),
                ThemeColorField::Foreground,
                appearance,
            ))
            .with_child(self.render_fill_editor(ThemeFillField::Accent, appearance))
            .with_child(self.render_color_field(
                crate::t!("theme-editor-outline"),
                ThemeColorField::Outline,
                appearance,
            ))
            .with_child(self.render_color_field(
                crate::t!("theme-editor-selection"),
                ThemeColorField::Selection,
                appearance,
            ));
        content = content.with_child(self.render_color_field(
            crate::t!("theme-editor-cursor"),
            ThemeColorField::Cursor,
            appearance,
        ));
        self.card(content.finish(), appearance)
    }

    fn render_fill_editor(
        &self,
        field: ThemeFillField,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let (label, start, end, mode_offset) = match field {
            ThemeFillField::Background => (
                crate::t!("theme-editor-background"),
                ThemeColorField::BackgroundStart,
                ThemeColorField::BackgroundEnd,
                0,
            ),
            ThemeFillField::Accent => (
                crate::t!("theme-editor-accent"),
                ThemeColorField::AccentStart,
                ThemeColorField::AccentEnd,
                3,
            ),
        };
        let current = self.fill_mode(field);
        let mut modes = Flex::row().with_spacing(4.0);
        for (offset, (mode, text)) in [
            (FillMode::Solid, crate::t!("theme-editor-solid")),
            (FillMode::Vertical, crate::t!("theme-editor-vertical")),
            (FillMode::Horizontal, crate::t!("theme-editor-horizontal")),
        ]
        .into_iter()
        .enumerate()
        {
            modes = modes.with_child(self.render_button(
                text,
                self.buttons.fill_modes[mode_offset + offset].clone(),
                if mode == current {
                    ButtonVariant::Accent
                } else {
                    ButtonVariant::Secondary
                },
                ThemeEditorBodyAction::SetFillMode(field, mode),
                appearance,
            ));
        }
        let colors = Flex::row()
            .with_spacing(8.0)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(self.render_color_control(start, appearance));
        let mut colors = colors;
        if current != FillMode::Solid {
            colors = colors.with_child(self.render_color_control(end, appearance));
        }
        Flex::column()
            .with_spacing(6.0)
            .with_child(self.label(label, appearance))
            .with_child(modes.finish())
            .with_child(colors.finish())
            .finish()
    }

    fn render_color_field(
        &self,
        label: String,
        field: ThemeColorField,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(Shrinkable::new(1.0, self.label(label, appearance)).finish())
            .with_child(self.render_color_control(field, appearance))
            .finish()
    }

    fn render_color_control(
        &self,
        field: ThemeColorField,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let key = field.key();
        let editor = self.color_editors.get(&key).expect("color editor exists");
        let color = self
            .color_values
            .get(&key)
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing color value"))
            .and_then(parse_theme_color_input)
            .unwrap_or_else(|_| appearance.theme().ui_error_color());
        let invalid = self.invalid_fields.contains(&key);
        Flex::row()
            .with_spacing(4.0)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background_color(color)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS)))
                        .with_border(
                            Border::all(if invalid { 2.0 } else { 1.0 }).with_border_fill(
                                if invalid {
                                    appearance.theme().ui_error_color().into()
                                } else {
                                    appearance.theme().outline()
                                },
                            ),
                        )
                        .finish(),
                )
                .with_width(28.0)
                .with_height(28.0)
                .finish(),
            )
            .with_child(self.render_text_input(editor, invalid, appearance))
            .finish()
    }

    fn render_text_input(
        &self,
        editor: &ViewHandle<EditorView>,
        invalid: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        TextInput::new(
            editor.clone(),
            UiComponentStyles {
                width: Some(EDITOR_WIDTH),
                height: Some(32.0),
                font_family_id: Some(appearance.monospace_font_family()),
                font_size: Some(appearance.ui_font_body()),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(CONTROL_RADIUS))),
                border_width: Some(if invalid { 2.0 } else { 1.0 }),
                border_color: Some(if invalid {
                    appearance.theme().ui_error_color().into()
                } else {
                    appearance.theme().outline().into()
                }),
                background: Some(Fill::None),
                padding: Some(Coords::uniform(6.0)),
                ..Default::default()
            },
        )
        .build()
        .finish()
    }

    fn render_ansi_colors(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut normal = Flex::column()
            .with_spacing(4.0)
            .with_child(self.label(crate::t!("theme-editor-ansi-normal"), appearance));
        let mut bright = Flex::column()
            .with_spacing(4.0)
            .with_child(self.label(crate::t!("theme-editor-ansi-bright"), appearance));
        for name in ANSI_NAMES {
            normal = normal.with_child(self.render_color_field(
                ansi_label(name),
                ThemeColorField::AnsiNormal(name),
                appearance,
            ));
            bright = bright.with_child(self.render_color_field(
                ansi_label(name),
                ThemeColorField::AnsiBright(name),
                appearance,
            ));
        }
        self.card(
            Flex::column()
                .with_spacing(8.0)
                .with_child(self.heading(crate::t!("theme-editor-ansi"), appearance))
                .with_child(normal.finish())
                .with_child(bright.finish())
                .finish(),
            appearance,
        )
    }

    fn render_ui_colors(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut content = Flex::column().with_spacing(8.0).with_child(
            Flex::row()
                .with_spacing(8.0)
                .with_child(self.render_button(
                    if self.ui_colors_expanded {
                        crate::t!("theme-editor-ui-hide")
                    } else {
                        crate::t!("theme-editor-ui-show")
                    },
                    self.buttons.ui_toggle.clone(),
                    ButtonVariant::Secondary,
                    ThemeEditorBodyAction::ToggleUiColors,
                    appearance,
                ))
                .with_child(self.render_button(
                    crate::t!("theme-editor-ui-auto"),
                    self.buttons.ui_reset.clone(),
                    ButtonVariant::Secondary,
                    ThemeEditorBodyAction::ResetUiColors,
                    appearance,
                ))
                .finish(),
        );
        if self.ui_colors_expanded {
            for name in UI_COLOR_FIELDS {
                content = content.with_child(self.render_color_field(
                    ui_color_label(name),
                    ThemeColorField::Ui(name),
                    appearance,
                ));
            }
        }
        self.card(content.finish(), appearance)
    }

    fn render_preview(&self, theme: &WarpTheme, appearance: &Appearance) -> Box<dyn Element> {
        let mut ansi = Flex::row().with_spacing(4.0);
        for name in ANSI_NAMES {
            ansi = ansi.with_child(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background_color(ansi_color(theme, false, name))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.0)))
                        .finish(),
                )
                .with_width(24.0)
                .with_height(12.0)
                .finish(),
            );
        }
        Container::new(
            Flex::column()
                .with_spacing(8.0)
                .with_child(
                    Text::new_inline(
                        crate::t!("theme-editor-preview-title"),
                        appearance.ui_font_family(),
                        appearance.ui_font_subheading(),
                    )
                    .with_color(theme.foreground().into_solid())
                    .finish(),
                )
                .with_child(
                    Text::new_inline(
                        crate::t!("theme-editor-preview-text"),
                        appearance.monospace_font_family(),
                        appearance.ui_font_body(),
                    )
                    .with_color(theme.foreground().into_solid())
                    .finish(),
                )
                .with_child(ansi.finish())
                .finish(),
        )
        .with_background(theme.background())
        .with_border(Border::all(1.0).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CARD_RADIUS)))
        .with_padding(Padding::uniform(16.0))
        .finish()
    }

    fn render_guard(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.card(
            Flex::column()
                .with_spacing(8.0)
                .with_child(self.heading(crate::t!("theme-editor-unsaved-title"), appearance))
                .with_child(self.label(crate::t!("theme-editor-unsaved-body"), appearance))
                .with_child(
                    Flex::row()
                        .with_spacing(8.0)
                        .with_child(self.render_button(
                            crate::t!("theme-editor-save"),
                            self.buttons.guard_save.clone(),
                            ButtonVariant::Accent,
                            ThemeEditorBodyAction::ResolveGuard(GuardDecision::Save),
                            appearance,
                        ))
                        .with_child(self.render_button(
                            crate::t!("theme-editor-discard"),
                            self.buttons.guard_discard.clone(),
                            ButtonVariant::Secondary,
                            ThemeEditorBodyAction::ResolveGuard(GuardDecision::Discard),
                            appearance,
                        ))
                        .with_child(self.render_button(
                            crate::t!("common-cancel"),
                            self.buttons.guard_cancel.clone(),
                            ButtonVariant::Secondary,
                            ThemeEditorBodyAction::ResolveGuard(GuardDecision::Cancel),
                            appearance,
                        ))
                        .finish(),
                )
                .finish(),
            appearance,
        )
    }

    fn heading(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Text::new_inline(
            text,
            appearance.ui_font_family(),
            appearance.ui_font_subheading(),
        )
        .with_color(appearance.theme().active_ui_text_color().into_solid())
        .finish()
    }

    fn label(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_body())
            .with_color(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into_solid(),
            )
            .finish()
    }

    fn card(&self, child: Box<dyn Element>, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(child)
            .with_background(appearance.theme().surface_1())
            .with_border(Border::all(1.0).with_border_fill(appearance.theme().outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CARD_RADIUS)))
            .with_padding(Padding::uniform(16.0))
            .finish()
    }
}

impl Entity for ThemeEditorBody {
    type Event = ThemeEditorBodyEvent;
}

impl View for ThemeEditorBody {
    fn ui_name() -> &'static str {
        "ThemeEditorBody"
    }

    fn on_window_closed(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_transient(ctx);
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        if self.draft.is_some() {
            self.render_editor(appearance)
        } else {
            ClippedScrollable::vertical(
                self.scroll.clone(),
                Container::new(self.render_template_selection(appearance))
                    .with_padding(Padding::uniform(16.0))
                    .finish(),
                ScrollbarWidth::Auto,
                appearance.theme().nonactive_ui_detail().into(),
                appearance.theme().active_ui_detail().into(),
                Fill::None,
            )
            .finish()
        }
    }
}

impl TypedActionView for ThemeEditorBody {
    type Action = ThemeEditorBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ThemeEditorBodyAction::ChooseTemplate(choice) => self.choose_template(*choice, ctx),
            ThemeEditorBodyAction::RequestClose => self.request_close(ctx),
            ThemeEditorBodyAction::RequestTemplates => {
                self.request_action(PendingAction::Templates, ctx)
            }
            ThemeEditorBodyAction::RequestImport => self.request_action(PendingAction::Import, ctx),
            ThemeEditorBodyAction::RequestImage => self.request_action(PendingAction::Image, ctx),
            ThemeEditorBodyAction::Export => self.export(ctx),
            ThemeEditorBodyAction::Save => {
                self.persist_draft(ctx);
            }
            ThemeEditorBodyAction::DiscardDraft => {
                self.execute_pending(PendingAction::Templates, ctx)
            }
            ThemeEditorBodyAction::ResolveGuard(decision) => self.resolve_guard(*decision, ctx),
            ThemeEditorBodyAction::SetFillMode(field, mode) => {
                self.set_fill_mode(*field, *mode, ctx)
            }
            ThemeEditorBodyAction::ToggleUiColors => {
                self.ui_colors_expanded = !self.ui_colors_expanded;
                ctx.notify();
            }
            ThemeEditorBodyAction::ResetUiColors => self.reset_ui_colors(ctx),
            ThemeEditorBodyAction::HandleImageSelected(path) => self.image_theme(path.clone(), ctx),
            ThemeEditorBodyAction::HandleImportSelected(path) => self.import_theme(path, ctx),
            ThemeEditorBodyAction::FilePickerCancelled => {
                self.image_loading = false;
                ctx.notify();
            }
        }
    }
}

const ANSI_NAMES: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

const UI_COLOR_FIELDS: [&str; 18] = [
    "surface_1",
    "surface_2",
    "surface_3",
    "border",
    "focus_border",
    "split_pane_border",
    "main_text",
    "sub_text",
    "hint_text",
    "disabled_text",
    "selection",
    "text_selection",
    "hover",
    "active",
    "warning",
    "error",
    "success",
    "link",
];

fn all_color_fields() -> Vec<ThemeColorField> {
    let mut fields = vec![
        ThemeColorField::BackgroundStart,
        ThemeColorField::BackgroundEnd,
        ThemeColorField::Foreground,
        ThemeColorField::AccentStart,
        ThemeColorField::AccentEnd,
        ThemeColorField::Outline,
        ThemeColorField::Selection,
        ThemeColorField::Cursor,
    ];
    for name in ANSI_NAMES {
        fields.push(ThemeColorField::AnsiNormal(name));
        fields.push(ThemeColorField::AnsiBright(name));
    }
    for name in UI_COLOR_FIELDS {
        fields.push(ThemeColorField::Ui(name));
    }
    fields
}

fn color_for_field(theme: &WarpTheme, field: ThemeColorField) -> ColorU {
    match field {
        ThemeColorField::BackgroundStart => fill_colors(theme.background()).0,
        ThemeColorField::BackgroundEnd => fill_colors(theme.background()).1,
        ThemeColorField::Foreground => theme.foreground().into_solid(),
        ThemeColorField::AccentStart => fill_colors(theme.accent()).0,
        ThemeColorField::AccentEnd => fill_colors(theme.accent()).1,
        ThemeColorField::Outline => theme.outline().into_solid(),
        ThemeColorField::Selection => theme.block_selection_color().into_solid(),
        ThemeColorField::Cursor => theme.cursor().into_solid(),
        ThemeColorField::AnsiNormal(name) => ansi_color(theme, false, name),
        ThemeColorField::AnsiBright(name) => ansi_color(theme, true, name),
        ThemeColorField::Ui(name) => effective_ui_color(theme, name),
    }
}

fn fill_colors(fill: ThemeFill) -> (ColorU, ColorU) {
    let first = fill.into_solid();
    let Ok(value) = serde_yaml::to_value(fill) else {
        return (first, first);
    };
    let mut colors = Vec::new();
    collect_yaml_colors(&value, &mut colors);
    (
        colors.first().copied().unwrap_or(first),
        colors.get(1).copied().unwrap_or(first),
    )
}

fn collect_yaml_colors(value: &serde_yaml::Value, colors: &mut Vec<ColorU>) {
    match value {
        serde_yaml::Value::String(value) => {
            if let Ok(color) = parse_theme_color_input(value) {
                colors.push(color);
            }
        }
        serde_yaml::Value::Sequence(values) => {
            for value in values {
                collect_yaml_colors(value, colors);
            }
        }
        serde_yaml::Value::Mapping(values) => {
            for (_, value) in values {
                collect_yaml_colors(value, colors);
            }
        }
        serde_yaml::Value::Null | serde_yaml::Value::Bool(_) | serde_yaml::Value::Number(_) => {}
    }
}

fn ansi_color(theme: &WarpTheme, bright: bool, name: &str) -> ColorU {
    let colors = if bright {
        &theme.terminal_colors().bright
    } else {
        &theme.terminal_colors().normal
    };
    match name {
        "black" => colors.black.into(),
        "red" => colors.red.into(),
        "green" => colors.green.into(),
        "yellow" => colors.yellow.into(),
        "blue" => colors.blue.into(),
        "magenta" => colors.magenta.into(),
        "cyan" => colors.cyan.into(),
        "white" => colors.white.into(),
        _ => ColorU::black(),
    }
}

fn effective_ui_color(theme: &WarpTheme, name: &str) -> ColorU {
    if let Some(color) = configured_ui_color(theme, name) {
        return color;
    }
    let background = theme.background();
    match name {
        "surface_1" => theme.surface_1().into_solid(),
        "surface_2" => theme.surface_2().into_solid(),
        "surface_3" => theme.surface_3().into_solid(),
        "border" => theme.outline().into_solid(),
        "focus_border" => theme.accent().into_solid(),
        "split_pane_border" => theme.split_pane_border_color().into_solid(),
        "main_text" => theme.main_text_color(background).into_solid(),
        "sub_text" => theme.sub_text_color(background).into_solid(),
        "hint_text" => theme.hint_text_color(background).into_solid(),
        "disabled_text" => theme.disabled_text_color(background).into_solid(),
        "selection" => theme.block_selection_color().into_solid(),
        "text_selection" => theme.text_selection_color().into_solid(),
        "hover" => theme.surface_2().into_solid(),
        "active" => theme.accent().into_solid(),
        "warning" => theme.ui_warning_color(),
        "error" => theme.ui_error_color(),
        "success" => theme.ui_green_color(),
        "link" => theme.accent().into_solid(),
        _ => theme.foreground().into_solid(),
    }
}

fn configured_ui_color(theme: &WarpTheme, name: &str) -> Option<ColorU> {
    let value = serde_yaml::to_value(theme).ok()?;
    let serde_yaml::Value::Mapping(root) = value else {
        return None;
    };
    let serde_yaml::Value::Mapping(ui_colors) =
        root.get(&serde_yaml::Value::String("ui_colors".into()))?
    else {
        return None;
    };
    let serde_yaml::Value::String(color) =
        ui_colors.get(&serde_yaml::Value::String(name.into()))?
    else {
        return None;
    };
    parse_theme_color_input(color).ok()
}

fn set_theme_fill(theme: &WarpTheme, key: &str, fill: ThemeFill) -> anyhow::Result<WarpTheme> {
    let mut value = serde_yaml::to_value(theme)?;
    set_yaml_path(&mut value, &[key], serde_yaml::to_value(fill)?);
    Ok(serde_yaml::from_value(value)?)
}

fn set_theme_color(theme: &WarpTheme, path: &[&str], color: ColorU) -> anyhow::Result<WarpTheme> {
    let mut value = serde_yaml::to_value(theme)?;
    set_yaml_path(
        &mut value,
        path,
        serde_yaml::Value::String(format_theme_color(color)),
    );
    Ok(serde_yaml::from_value(value)?)
}

fn set_theme_image_path(theme: &WarpTheme, path: &Path) -> anyhow::Result<WarpTheme> {
    let mut value = serde_yaml::to_value(theme)?;
    set_yaml_path(
        &mut value,
        &["background_image", "path"],
        serde_yaml::Value::String(path.to_string_lossy().into_owned()),
    );
    Ok(serde_yaml::from_value(value)?)
}

fn remove_theme_key(theme: &WarpTheme, key: &str) -> anyhow::Result<WarpTheme> {
    let mut value = serde_yaml::to_value(theme)?;
    if let serde_yaml::Value::Mapping(root) = &mut value {
        root.remove(&serde_yaml::Value::String(key.into()));
    }
    Ok(serde_yaml::from_value(value)?)
}

fn reset_ui_color_overrides(theme: &WarpTheme) -> anyhow::Result<WarpTheme> {
    let outline = theme.outline().into_solid();
    let selection = theme.block_selection_color().into_solid();
    let theme = remove_theme_key(theme, "ui_colors")?;
    let theme = set_theme_color(&theme, &["ui_colors", "border"], outline)?;
    set_theme_color(&theme, &["ui_colors", "selection"], selection)
}

fn set_yaml_path(root: &mut serde_yaml::Value, path: &[&str], value: serde_yaml::Value) {
    let Some((head, tail)) = path.split_first() else {
        *root = value;
        return;
    };
    if !matches!(root, serde_yaml::Value::Mapping(_)) {
        *root = serde_yaml::Value::Mapping(Default::default());
    }
    let serde_yaml::Value::Mapping(map) = root else {
        return;
    };
    let key = serde_yaml::Value::String((*head).into());
    if tail.is_empty() {
        map.insert(key, value);
    } else {
        let child = map
            .entry(key)
            .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
        set_yaml_path(child, tail, value);
    }
}

fn safe_theme_filename(name: &str) -> String {
    let mut filename = String::new();
    for character in name.trim().chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_') {
            filename.push(character);
        } else if character.is_whitespace() && !filename.ends_with('-') {
            filename.push('-');
        }
    }
    filename.trim_matches('-').to_owned()
}

fn ansi_label(name: &str) -> String {
    match name {
        "black" => crate::t!("theme-editor-color-black"),
        "red" => crate::t!("theme-editor-color-red"),
        "green" => crate::t!("theme-editor-color-green"),
        "yellow" => crate::t!("theme-editor-color-yellow"),
        "blue" => crate::t!("theme-editor-color-blue"),
        "magenta" => crate::t!("theme-editor-color-magenta"),
        "cyan" => crate::t!("theme-editor-color-cyan"),
        "white" => crate::t!("theme-editor-color-white"),
        _ => name.to_owned(),
    }
}

fn ui_color_label(name: &str) -> String {
    match name {
        "surface_1" => crate::t!("theme-editor-ui-surface-1"),
        "surface_2" => crate::t!("theme-editor-ui-surface-2"),
        "surface_3" => crate::t!("theme-editor-ui-surface-3"),
        "border" => crate::t!("theme-editor-ui-border"),
        "focus_border" => crate::t!("theme-editor-ui-focus-border"),
        "split_pane_border" => crate::t!("theme-editor-ui-split-pane-border"),
        "main_text" => crate::t!("theme-editor-ui-main-text"),
        "sub_text" => crate::t!("theme-editor-ui-sub-text"),
        "hint_text" => crate::t!("theme-editor-ui-hint-text"),
        "disabled_text" => crate::t!("theme-editor-ui-disabled-text"),
        "selection" => crate::t!("theme-editor-ui-selection"),
        "text_selection" => crate::t!("theme-editor-ui-text-selection"),
        "hover" => crate::t!("theme-editor-ui-hover"),
        "active" => crate::t!("theme-editor-ui-active"),
        "warning" => crate::t!("theme-editor-ui-warning"),
        "error" => crate::t!("theme-editor-ui-error"),
        "success" => crate::t!("theme-editor-ui-success"),
        "link" => crate::t!("theme-editor-ui-link"),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
#[path = "theme_editor_body_tests.rs"]
mod tests;
