//! Modul levl documenatation.
//!
//! Details are full fo errors.

mod simple;

mod enumerate;

// Shud be chcked now
// Verify **some** _super_ *duper* [markdown](https://ahoi.io/).
struct X;

/*
 * Also check thiz one
 */
impl X {
	/// New, as in new. But also not.
	///
	/// Half sentence for X #2.
	fn new() -> Self {
		unimplemented!()
	}

	/// Old, as in really old.
	///
	/// But what does "old" really mean?
	fn old(&self) {
		unimplemented!()
	}
}
