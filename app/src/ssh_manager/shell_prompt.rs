//! Shell prompt detection. Used by SSH injectors (`secret_injector`, `startup_command_injector`,
//! `su_password_injector`) to trigger actions only after login completes and shell is ready.
//!
//! Only examines last 256 bytes of buffer, matching common prompt endings:
//! - ASCII: `$ ` / `# ` / `> `
//! - Common powerline / Starship symbols: ❯  ▶  »  λ  →

const TAIL_BYTES: usize = 256;

/// Check if buffer tail matches shell prompt pattern.
pub fn bytes_look_like_shell_prompt(bytes: &[u8]) -> bool {
    let tail = if bytes.len() > TAIL_BYTES {
        &bytes[bytes.len() - TAIL_BYTES..]
    } else {
        bytes
    };
    if tail.ends_with(b"$ ") || tail.ends_with(b"# ") || tail.ends_with(b"> ") {
        return true;
    }
    // Multibyte prompt symbol + space
    if tail.ends_with(&[0xe2, 0x9d, 0xaf, 0x20])  // ❯
        || tail.ends_with(&[0xe2, 0x96, 0xb6, 0x20])  // ▶
        || tail.ends_with(&[0xc2, 0xbb, 0x20])  // »
        || tail.ends_with(&[0xce, 0xbb, 0x20])  // λ
        || tail.ends_with(&[0xe2, 0x86, 0x92, 0x20])  // →
    {
        return true;
    }
    false
}

#[cfg(test)]
#[path = "shell_prompt_tests.rs"]
mod tests;
