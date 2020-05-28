//! Modul levl documenatation.
//!
//! Details are full fo errors.

mod simple;

/// Secret weapon X.
///
/// Somethign very secret but also not,
/// lets continuae thas on a nwe
/// line.
///
/// A seprate paragraphius.
enum X {
	/// A nice instroment.
	Xylophon,
	/// Another rythmuic instrment.
	BongoDrums,
}



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


// Should not be checked for now
// Verify **some** _super_ *duper* [markdown](https://ahoi.io/).