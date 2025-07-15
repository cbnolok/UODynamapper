use bevy::math::Rect;
use bevy::prelude::Vec2;

// Bevy Rect is rectangle defined by two opposite corners.
// This merges the rectangles into a greater one.
pub fn bounding_rect(rects: &[Rect]) -> Option<Rect> {
    // Early return for empty input
    let mut rects = rects.iter();
    let first = rects.next()?;

    let mut min_x = first.min.x;
    let mut min_y = first.min.y;
    let mut max_x = first.max.x;
    let mut max_y = first.max.y;

    for rect in rects {
        min_x = min_x.min(rect.min.x);
        min_y = min_y.min(rect.min.y);
        max_x = max_x.max(rect.max.x);
        max_y = max_y.max(rect.max.y);
    }

    Some(Rect {
        min: Vec2::new(min_x, min_y),
        max: Vec2::new(max_x, max_y),
    })
}
