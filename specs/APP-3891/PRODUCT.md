# CLI Agent Image Paste

## Summary

Allow users to paste screenshots into the CLI agent rich input (e.g. when composing prompts for Claude Code, Antigravity, Codex) and have them delivered to the CLI agent as images. Images appear as removable thumbnail attachments in the rich input before submission, so the user can verify the actual image rather than seeing an implementation path.

## Problem

When using CLI agents like Claude Code through Warp's rich input, users cannot share visual context (screenshots, UI mockups, error dialogs) with the agent. The only workaround is to save the image to disk, note the file path, and manually type a reference to it — which breaks the conversational flow.

Warp's agent mode already supports pasting images as attachments, but the CLI agent rich input did not provide a visual preview or deliver images on submission.

## Goals

- Users can paste screenshots into the CLI agent rich input and see them as removable image thumbnails with filenames.
- On submission, attached images are delivered to the CLI agent so it can actually see them.
- The attachment UX (add, remove, limits) matches existing agent mode behavior, with a visual preview in the CLI-agent composer.
- Works with any CLI agent that supports clipboard image paste out of the box.

## Non-goals

- Drag-and-drop image files into the CLI agent rich input (should work for free via existing infrastructure, but not explicitly targeted or tested).
- Supporting CLI agents that cannot read images from the clipboard through their platform-native paste sequence.
- Sending images to Warp's own agent mode backend through this path (that uses a separate server-side flow).

## Figma

Figma: none provided. The compact thumbnail treatment is specified in `docs/ui/cli-agent-image-attachment.html` and reuses the existing attachment/remove interaction.

## User experience

### Attaching images

1. User opens the CLI agent rich input (Ctrl+G or footer button) while a CLI agent is running.
2. User takes a screenshot or copies an image to the clipboard.
3. User pastes (Cmd+V / Ctrl+V) into the rich input.
4. A compact image thumbnail appears above the editor, with the filename (e.g. `pasted-image-1713121234.png`) and an × button to remove it.
5. Multiple images can be pasted. Each appears as a separate thumbnail attachment. The same per-query and per-conversation limits from agent mode apply.

### Removing images

- Clicking the × on an attachment removes that image. This uses the existing `DeleteAttachment` action — identical to agent mode.

### Submitting with images

- When the user submits the prompt (Enter), images are delivered to the CLI agent first, followed by the text prompt.
- The image thumbnails disappear after submission.
- If no images are attached, submission behaves exactly as before.

### Delivery mechanism

Images are delivered by simulating what a user would do manually: for each attached image, Warp writes the image data to the system clipboard and sends the platform-native image-paste sequence to the PTY (`Ctrl+V` on macOS/Linux, `Alt+V` on Windows). The CLI agent (e.g. Claude Code) reads the image from the clipboard natively.

- A 500ms delay is inserted between each image paste to give the CLI agent time to read from the clipboard before it's overwritten with the next image. This was tested empirically in prototype - we need a relatively significant delay here for the CLI agent to pick up the paste correctly.
- After all images are pasted, the text prompt is sent using the agent-specific submission strategy (inline, bracketed paste, or delayed enter).

## Alternate approaches considered

### 1. Save images to temp files and include file paths in the prompt text

The first approach implemented was to decode each attached image from base64, write it to a temp file on disk (e.g. `/var/folders/.../warp-cli-image-1776199745040956000.png`), and prepend a `[Attached images: /path/to/file]` block to the prompt text.

**Why we didn't choose this:**
- The temp file paths are ugly and OS-specific (`/var/folders/...` on macOS).
- CLI agent then has to go read the files from given paths.
- Requires cleanup logic (tracking temp files, deleting on session end).
- Not how a user would naturally share an image with a CLI agent.

### 2. Don't intercept image paste at all — let the native paste sequence pass through to the PTY

Instead of showing thumbnails, let the paste keypress go directly to the CLI agent's PTY so its own image handling takes over.

**Why we didn't choose this:**
- Loses the thumbnail UI entirely — no visual confirmation before submission, no ability to remove an accidentally pasted image, no multi-image staging.

### 3. Encode images inline in the prompt (base64 or data URI)

Embed the image data directly in the prompt text sent to the PTY.

**Not considered seriously because:**
- No CLI agent parses inline base64 image data from stdin.
- Would produce enormous, unreadable prompt text.

## Success criteria

1. Pasting a screenshot (Cmd+V) into the CLI agent rich input produces an image thumbnail above the editor and inserts no file path into the text.
2. The attachment shows the actual image, its filename, and an × close button.
3. Clicking × removes the thumbnail and the underlying pending attachment.
4. Submitting with one attached image: Claude Code shows `[Image #1]` in its prompt and can describe the image content.
5. Submitting with two attached images: Claude Code shows `[Image #1] [Image #2]` and can distinguish between them.
6. Submitting with no attached images behaves identically to the previous behavior (no regression).
7. Image attachment limits (per-query and per-conversation) are enforced, with toast messages for excess images.

## Validation

- **Manual test — single image**: Paste a screenshot, type a prompt referencing the image, submit. Verify Claude Code sees and describes the image correctly.
- **Manual test — multiple images**: Paste two different screenshots, submit. Verify Claude Code sees both as distinct images (`[Image #1]` and `[Image #2]`).
- **Manual test — remove thumbnail**: Paste an image, click ×, submit. Verify no image is sent to the CLI agent.
- **Manual test — no images**: Submit a text-only prompt. Verify behavior is unchanged.
- **Manual test — limits**: Paste more images than the per-query limit. Verify a toast appears and excess images are not attached.
- **Build verification**: `cargo fmt` and `cargo clippy` pass with no warnings.

## Open questions

- **Delay tuning**: The 500ms delay between image pastes is sufficient for Claude Code but may need adjustment for other CLI agents. Need to explore.
- **Non-image-paste agents**: For CLI agents that do not support native clipboard image paste, should the composer reject the attachment explicitly? A visible file-path fallback is not permitted.
Need to check if this applies to any CLI agents.
