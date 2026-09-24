//! Disambiguate only colliding terminal identities without assigning row-based names.

/// Hash every byte of the persistent session identity. This is a display hint,
/// never a routing key; even a hash collision must keep both labels distinct.
fn suffix(session: &[u8]) -> String {
    let hash = session.iter().fold(0x811c9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    });
    format!("{hash:08x}")
}

pub(super) fn resolve(identities: &[(Vec<u8>, String, String)]) -> Vec<String> {
    identities
        .iter()
        .map(|(session, short, full)| {
            if identities
                .iter()
                .filter(|(_, other, _)| other == short)
                .count()
                < 2
            {
                return short.clone();
            }
            if identities
                .iter()
                .filter(|(_, _, other)| other == full)
                .count()
                < 2
            {
                return full.clone();
            }
            let mut discriminator = suffix(session);
            if identities.iter().any(|(other_session, _, other_full)| {
                other_session != session
                    && other_full == full
                    && suffix(other_session) == discriminator
            }) {
                discriminator = session.iter().map(|byte| format!("{byte:02x}")).collect();
            }
            format!("{full} · {discriminator}")
        })
        .collect()
}

#[cfg(test)]
#[path = "terminal_titles_tests.rs"]
mod tests;
