pub trait Between: PartialOrd + Sized {
    fn between(&self, min: Self, max: Self) -> bool {
        *self >= min && *self <= max
    }
}

// Implement for all types matching our bounds (PartialOrd is implemented for all numbers)
impl<T: PartialOrd + Sized> Between for T {}
