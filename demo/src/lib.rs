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
