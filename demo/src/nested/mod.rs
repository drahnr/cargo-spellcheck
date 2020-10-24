

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
    #[doc=r###"And a different interesting thing
    because we have a random newline here?!"###]
    third: String,
}

/// A long documentation which is short enough for two lines
/// but too long for one line.
struct TooLong;

/// And these lines are too short so they become just two lines
/// instead of three, as it was
/// initially.
struct TooShort;

#[ doc = "A long comment which we wanna reflow. So it's Saturday, are you having any plans for tonight?" ]
struct Someodo;

#[ doc= r#"A long comment which we wanna reflow. So it's Saturday, are you having any plans for 
           tonight? We're gonna end up with three lines here I think."#]
struct AnotherSomeodo;

#[ doc= r#"A long short
comment which we wanna reflow
to one line."#]
struct AnotherSomeodo2;
