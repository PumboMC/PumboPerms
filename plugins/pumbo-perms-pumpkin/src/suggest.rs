//! Where a Tab suggestion goes in the typed line.

/// Byte offset and length of the last word of `input`, the whole line the
/// client typed. Pumpkin 0.2 replaces only this word, so suggestions are bare
/// values. Words are split on plain spaces, like `split_args`.
pub fn last_word(input: &str) -> (usize, usize) {
    let start = input.rfind(' ').map_or(0, |i| i + 1);
    (start, input.len() - start)
}

#[cfg(test)]
mod tests {
    use super::last_word;

    #[test]
    fn suggestion_replaces_only_the_last_word() {
        assert_eq!(last_word("/pp user "), (9, 0));
        assert_eq!(last_word("/pp user Qh"), (9, 2));
        assert_eq!(last_word("/pp user Qhash parent add "), (26, 0));
        assert_eq!(last_word("/pp  group  vi"), (12, 2));
    }
}
