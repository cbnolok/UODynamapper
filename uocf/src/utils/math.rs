/*  Generic version not working... yet?
extern crate num;

pub fn trunc_downcast<F, T>(number_from: F) -> T
where
    F: num::traits::Num + std::cmp::PartialOrd + std::cmp::Ord + num::traits::Bounded,
    T: num::traits::Num + std::cmp::PartialOrd + std::cmp::Ord + num::traits::Bounded
{
    assert!(T::max_value() < F::max_value());
    std::cmp::max(T::max_value() as F, number_from) as T
}
 */

// If the usize stype is smaller than u64 (e.g. under x86, 32 bits arch),
//  then return usize maximum value instead of truncating the number upper bytes.
pub fn downcast_ceil_usize(from: u64) -> usize
{
    if from > usize::MAX as u64 {
        eprintln!("Warning: downcasting u64 to usize required ceiling the value to usize maximum value");
        usize::MAX
    } else {
        from as usize
    }
}
