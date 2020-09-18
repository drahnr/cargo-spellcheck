

mod justone;
mod justtwo;
mod fragments;
mod again;

/// Nested;
struct Nest;

/// Overly long statements that should be reflown since they are __very__ long and exceed the line limit.
///
/// This struct has a lot of documentation but unfortunately, the lines are just too long.
struct SomeLong {
    /// This member is interesting though since it has some indentation. These whitespaces must be kept.
    member: i8,
    #[ doc = "This member is interesting though since it has some indentation. These whitespaces must be kept."]
    sec: i8,
}

/// A long documentation which is short enough for two lines
/// but too long for one line.
struct TooLong;
