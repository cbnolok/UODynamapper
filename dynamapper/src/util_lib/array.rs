use bevy::math::prelude::*;

/// Get the tile layer value from a packed [UVec4] array.
#[inline]
pub fn uvec4_elem_get_mut(vec: &mut [UVec4], idx: usize) -> &mut u32 {
    // Equivalent to idx / 4
    let block = idx >> 2;
    // Equivalent to idx % 4
    let offset = idx & 3;
    &mut vec[block][offset]
}
/*
pub fn uvec4_elem_get_ref(vec: & [UVec4], idx: usize) -> & u32 {
    let block = idx / 4;
    let offset = idx % 4;
    & vec[block][offset]
}
*/

//pub fn get_1d_array_index_as_2d REF <'a, T> (arr: &'a [T], tx: usize, ty: usize) -> &'a T {
//    & arr[ty * CHUNK_SIZE + tx]
//}
#[inline]
pub fn get_1d_array_index_as_2d(first_dim_size: usize, tx: usize, ty: usize) -> usize {
    ty * first_dim_size + tx
}
