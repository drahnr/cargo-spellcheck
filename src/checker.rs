//! The desired error output should look like this:
//!
//! ```raw
//! error[spellcheck]: Spelling
//! --> src/main.rs:138:16
//!     |
//! 138 | /// Thisf module is for easing the pain with printing text in the terminal.
//!     |     ^^^^^
//!     |     - The word "Thisf" is not in our dictionary. If you are sure this spelling is correct,
//!     |     - you can add it to your personal dictionary to prevent future alerts.
//! ```



pub struct Extractor;