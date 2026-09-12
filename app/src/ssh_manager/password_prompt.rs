use lazy_static::lazy_static;
use regex::bytes::Regex;

const PASSWORD_PROMPT_PATTERN: &str = r"(?im)^(?:\[[^\]\r\n]+\]\s+password for [^:\r\n]+|\S+@\S+(?:'s)?\s+password|password(?: for [^:\r\n]+)?|enter passphrase for key [^:\r\n]+):[ \t\r]*$";

lazy_static! {
    static ref PASSWORD_PROMPT_REGEX: Regex =
        Regex::new(PASSWORD_PROMPT_PATTERN).expect("password prompt regex must compile");
}

pub fn bytes_look_like_password_prompt(bytes: &[u8]) -> bool {
    PASSWORD_PROMPT_REGEX.is_match(bytes)
}

#[cfg(test)]
#[path = "password_prompt_tests.rs"]
mod tests;
