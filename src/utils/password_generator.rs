use rand::seq::SliceRandom;

const LOWERCASE: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPERCASE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
const SPECIAL: &[u8] = b"!@#$?/-_";

pub fn generate_password() -> String {
    let mut rng = rand::thread_rng();
    let mut password: Vec<u8> = Vec::with_capacity(14);

    // Guarantee minimums: 1 lowercase, 1 uppercase, 4 digits, 2 special
    password.push(*LOWERCASE.choose(&mut rng).unwrap());
    password.push(*UPPERCASE.choose(&mut rng).unwrap());
    for _ in 0..4 {
        password.push(*DIGITS.choose(&mut rng).unwrap());
    }
    for _ in 0..2 {
        password.push(*SPECIAL.choose(&mut rng).unwrap());
    }

    // Fill remaining 6 characters from the full charset
    let full_charset: Vec<u8> = [LOWERCASE, UPPERCASE, DIGITS, SPECIAL].concat();
    for _ in 0..6 {
        password.push(*full_charset.choose(&mut rng).unwrap());
    }

    // Shuffle so the guaranteed characters aren't always at the start
    password.shuffle(&mut rng);

    String::from_utf8(password).unwrap()
}