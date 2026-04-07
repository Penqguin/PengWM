//! Geometry primitives for window positioning and sizing.

use euclid::{default::Rect as EuclidRect, default::Point2D, default::Size2D};

/// A rectangle representing a window's position and size.
pub type Rect = EuclidRect<i32>;

/// A 2D point in screen coordinates.
pub type Point = Point2D<i32>;

/// A 2D size (width and height).
pub type Size = Size2D<i32>;
