use crate::db::cache::Snapshot;
use rand::Rng;

// ---------------------------------------------------------------------------
// Cache helpers — these used to be per-request `$sample` queries.
// ---------------------------------------------------------------------------

fn word(snap: &Snapshot) -> String {
    snap.word().unwrap_or("cool").to_string()
}

fn domain(snap: &Snapshot) -> String {
    snap.domain().unwrap_or("gmail.com").to_string()
}

// ---------------------------------------------------------------------------
// Pure formatting helpers (testable without DB)
// ---------------------------------------------------------------------------

/// john.smith
pub fn fmt_dot(first: &str, last: &str) -> String {
    format!("{}.{}", first, last)
}

/// john_smith
pub fn fmt_underscore(first: &str, last: &str) -> String {
    format!("{}_{}", first, last)
}

/// johnsmith98 / johnsmith2003
pub fn fmt_name_year(first: &str, last: &str, year: u32) -> String {
    format!("{}{}{}", first, last, year)
}

/// johnblue
pub fn fmt_name_word(first: &str, word: &str) -> String {
    format!("{}{}", first, word)
}

/// john42
pub fn fmt_name_digits(first: &str, digits: u32) -> String {
    format!("{}{}", first, digits)
}

/// john.smith42
pub fn fmt_dot_digits(first: &str, last: &str, digits: u32) -> String {
    format!("{}.{}{}", first, last, digits)
}

/// cooljohn
pub fn fmt_word_name(word: &str, first: &str) -> String {
    format!("{}{}", word, first)
}

/// johnm42 / alexs123
pub fn fmt_name_initial_digits(first: &str, last_initial: char, digits: u32) -> String {
    format!("{}{}{}", first, last_initial, digits)
}

/// j.smith  (initial dot lastname)
pub fn fmt_initial_dot_last(first: &str, last: &str) -> String {
    let initial = first.chars().next().unwrap_or('x');
    format!("{}.{}", initial, last)
}

/// jsmith  (initial + lastname)
pub fn fmt_initial_last(first: &str, last: &str) -> String {
    let initial = first.chars().next().unwrap_or('x');
    format!("{}{}", initial, last)
}

/// smith_john  (lastname_firstname)
pub fn fmt_last_underscore_first(first: &str, last: &str) -> String {
    format!("{}_{}", last, first)
}

/// john.smith.42
pub fn fmt_dot_dot_digits(first: &str, last: &str, digits: u32) -> String {
    format!("{}.{}.{}", first, last, digits)
}

/// cool_john42  (word_firstname + digits)
pub fn fmt_word_underscore_name_digits(word: &str, first: &str, digits: u32) -> String {
    format!("{}_{}{}", word, first, digits)
}

/// johnsmith_cool  (firstname + lastname + _ + word)
pub fn fmt_name_word_suffix(first: &str, last: &str, word: &str) -> String {
    format!("{}{}_{}", first, last, word)
}

/// Generate a username using the provided first/last names instead of fetching new ones.
/// Words come from the in-memory snapshot, so this no longer touches MongoDB.
pub fn generate_username_from(snap: &Snapshot, first: &str, last: &str) -> String {
    let (pattern, year, d2, d3) = {
        let mut rng = rand::thread_rng();
        (
            rng.gen_range(0..14u32),
            if rng.gen_bool(0.5) { rng.gen_range(90..99u32) } else { rng.gen_range(1990..2010u32) },
            rng.gen_range(10..99u32),
            rng.gen_range(10..999u32),
        )
    };

    match pattern {
        0  => fmt_dot(first, last),
        1  => fmt_underscore(first, last),
        2  => fmt_name_year(first, last, year),
        3  => fmt_name_word(first, &word(snap)),
        4  => fmt_name_digits(first, d2),
        5  => fmt_dot_digits(first, last, d2),
        6  => fmt_word_name(&word(snap), first),
        7  => {
            let initial = last.chars().next().unwrap_or('s');
            fmt_name_initial_digits(first, initial, d3)
        }
        8  => fmt_initial_dot_last(first, last),
        9  => fmt_initial_last(first, last),
        10 => fmt_last_underscore_first(first, last),
        11 => fmt_dot_dot_digits(first, last, d2),
        12 => fmt_word_underscore_name_digits(&word(snap), first, d2),
        _  => fmt_name_word_suffix(first, last, &word(snap)),
    }
}

/// Generate an email using pre-fetched first/last names.
pub fn generate_email_from(snap: &Snapshot, first: &str, last: &str) -> String {
    let username = generate_username_from(snap, first, last);
    format!("{}@{}", username.to_lowercase(), domain(snap))
}