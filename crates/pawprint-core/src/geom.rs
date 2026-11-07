//! Physical-pixel geometry. All core coordinates are physical pixels in the
//! backend's desktop coordinate space; backends convert at the edge.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PhysicalRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn center(&self) -> Point {
        Point::new(
            self.x + self.width as i32 / 2,
            self.y + self.height as i32 / 2,
        )
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Move inside `area`. When the rect is larger than `area`, its top-left
    /// corner is kept inside instead.
    pub fn clamped_into(self, area: PhysicalRect) -> PhysicalRect {
        let fit = |pos: i32, len: u32, lo: i32, area_len: u32| -> i32 {
            let hi = lo + area_len as i32 - len as i32;
            pos.min(hi).max(lo)
        };
        PhysicalRect {
            x: fit(self.x, self.width, area.x, area.width),
            y: fit(self.y, self.height, area.y, area.height),
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamping() {
        let area = PhysicalRect::new(0, 30, 1920, 1050);
        let cases = [
            (
                PhysicalRect::new(100, 100, 64, 64),
                PhysicalRect::new(100, 100, 64, 64),
            ),
            (
                PhysicalRect::new(1900, 100, 64, 64),
                PhysicalRect::new(1856, 100, 64, 64),
            ),
            (
                PhysicalRect::new(-5, 0, 64, 64),
                PhysicalRect::new(0, 30, 64, 64),
            ),
            (
                PhysicalRect::new(10, 2000, 64, 64),
                PhysicalRect::new(10, 1016, 64, 64),
            ),
            (
                PhysicalRect::new(10, 10, 4000, 64),
                PhysicalRect::new(0, 30, 4000, 64),
            ),
        ];
        for (rect, want) in cases {
            assert_eq!(rect.clamped_into(area), want, "{rect:?}");
        }
    }

    #[test]
    fn rect_basics() {
        let r = PhysicalRect::new(10, 20, 30, 40);
        assert_eq!(
            (r.right(), r.bottom(), r.center()),
            (40, 60, Point::new(25, 40))
        );
        assert!(r.contains(Point::new(10, 20)) && !r.contains(Point::new(40, 20)));
    }
}
