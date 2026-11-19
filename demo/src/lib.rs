//! Fancy module docs are really helpful if they contain usage examples.

/// Pick option a also known as door #1.
pub fn a() {

}


#[doc = "Pick option b also known as door #2."]
pub fn b() {

}

#[doc = r##"Pick option c also known as door #3."##]
pub fn c() {

}

#[doc = r#"Risk not ya ting?"#]
pub fn take_the_money_and_leave() {

}


/// Possible ways to run rustc and request various parts of LTO.
///
/// Variant            | Flag                   | Object Code | Bitcode
/// -------------------|------------------------|-------------|--------
/// `Run`              | `-C lto=foo`           | n/a         | n/a
/// `Off`              | `-C lto=off`           | n/a         | n/a
/// `OnlyBitcode`      | `-C linker-plugin-lto` |             | ✓
/// `ObjectAndBitcode` |                        | ✓           | ✓
/// `OnlyObject`       | `-C embed-bitcode=no`  | ✓           |
pub fn exploding_complexity() {

}

/// # Examples
///
#[cfg_attr(unix, doc = "```no_run")]
#[cfg_attr(not(unix), doc = "```ignore")]
/// use std::path::Path;
/// assert!(Path::new("./is_a_directory/").is_dir());
/// ```
///
/// More explanation after the example.
pub struct Directory;

/// Select a transport for the current platform.
#[cfg_attr(unix, doc = "Local sockets carry mesages between processes.")]
#[cfg_attr(not(unix), doc = r#"Network sockets accept remote conectons."#)]
pub struct PlatformTransport;

/// Configure the local transfer buffer.
#[cfg(unix)]
#[cfg_attr(
    unix,
    cfg_attr(target_pointer_width = "64", doc = "Large buffers improve performnce."),
    doc = "Socket permisions control access."
)]
pub struct LocalTransfer;

#[doc = r#"Read the delivery adress before sending a parcel.

Keep the tracking number for future refernce.
"#]
pub struct DeliveryLabel;

/// Return literal documentation that the spellchecker deliberately does not expand.
macro_rules! i_am_unsupported {
    ($text:literal) => {
        $text
    };
}

// Selecting an unsupported macro skips this owner's documentation with a warning.
#[doc = i_am_unsupported!("fun")]
pub struct MacroDocumentation;

// Only the Unix alternative selects the unsupported macro; the other remains checkable.
/// Retry the transfer after a short delai.
#[cfg_attr(unix, doc = i_am_unsupported!("fun"))]
#[cfg_attr(not(unix), doc = "Reconnect after an interuption.")]
pub struct ConditionalMacroDocumentation;
