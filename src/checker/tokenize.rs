static DEFAULT_TOKENIZER_BYTES: &[u8] =
include_bytes!(concat!(env!("OUT_DIR"), "/en_tokenizer.bin"));

static DEFAULT_RULES_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/en_rules.bin"));

/// Returns absolute offsets and the data with the token in question.
///
/// Does not handle hyphenation yet or partial words at boundaries.
/// Returns the a vector of ranges for the input str.
///
/// All ranges are in characters.
fn tokenize_naive(s: &str, char_offset: usize, splitchars: &str) -> Vec<Range> {
    let mut started = false;
    // in characters
    let mut linear_start = 0;
    let mut linear_end;

    // very few sentences have more than 32 words, hence ranges.
    let mut bananasplit = Vec::with_capacity(32);

    let is_split_char = move |c: char| { c.is_whitespace() || splitchars.contains(c) };

    for (c_idx, (_byte_offset, c)) in s.char_indices().enumerate() {
        if is_split_char(c) {
            linear_end = c_idx;
            if started {
                let range = Range {
                    start: linear_start + char_offset,
                    end: linear_end + char_offset,
                };
                bananasplit.push(range);
            }
            started = false;
        } else {
            if !started {
                linear_start = c_idx;
                started = true;
            }
        }
    }

    // at the end of string, assume word complete
    // TODO for hypenation, check if line ends with a dash
    if started {
        if let Some((idx, _)) = s.char_indices().next_back() {
            // increase by one, since the range's end goes one beyond, end bounds is _exclusive_ for ranges
            let linear_end = idx + 1;
            bananasplit.push(linear_start..linear_end)
        } else {
            log::error!("BUG: Most likely lost a word when tokenizing!");
        }
    }
    bananasplit
}

/// Recommeneded default split chars for intra sentence spliting:
/// `splitchars = "\";:,?!#(){}[]\n\r/`"`.
fn tokenize(s: &str, splitchars: &str) -> Result<Vec<Range>> {
    use std::{fs, str::FromStr};
    use srx::SRX;

    let srx = SRX::from_str(&fs::read_to_string("data/segment.srx")?)?;
    let english_rules = srx.language_rules("en_US");

    let previous_end = 0;
    let mut char_counter = previous_end;
    let mut acc = Vec::new();

    for byte_range in english_rules.split_ranges(s) {
        char_counter += s[previous_end..=(byte_range.start-1)].chars().count();
        acc.extend(tokenize_naive(&s[byte_range], char_counter, splitchars));
    }

    Ok(acc)
}

use nlprule::{Rules, Tokenizer};

fn tokenizer() -> Tokenizer {

    lazy_static! {
        static ref TOKENIZER: Tokenizer = {
            info!("Loading tokenizer...");
            let tokenizer = config
            .override_tokenizer
            .as_ref()
            .map_or_else(
                || {
                    Ok(Tokenizer::from_reader(&mut &*DEFAULT_TOKENIZER_BYTES)
                        .expect("build.rs pulls valid tokenizer description. qed"))
                },
                |path| -> Result<Tokenizer> {
                    let f = fs::File::open(&path)?;
                    Ok(Tokenizer::from_reader(f)?)
                },
            )?;
            info!("Loaded tokenizer.");
        };
    }

    TOKENIZER
}


fn rules() -> {
    info!("Loading rules...");

    let rules = config.override_rules.as_ref().map_or_else(
    || {
        Ok(Rules::from_reader(&mut &*DEFAULT_RULES_BYTES)
            .expect("build.rs pulls valid rules set. qed"))
    },
    |path| -> Result<Rules> {
        let f = fs::File::open(&path)?;
        Ok(Rules::from_reader(f)?)
    },
    )?;
    info!("Loaded rules.");

}
