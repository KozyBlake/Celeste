//! Port of `net.sybyline.scarlet.util.Maths`.

pub fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    if value < min {
        min
    } else if max < value {
        max
    } else {
        value
    }
}

pub fn clamp_i64(value: i64, min: i64, max: i64) -> i64 {
    if value < min {
        min
    } else if max < value {
        max
    } else {
        value
    }
}

pub fn clamp_f32(value: f32, min: f32, max: f32, nan: f32) -> f32 {
    if value < min {
        min
    } else if max < value {
        max
    } else if value.is_nan() {
        nan
    } else {
        value
    }
}

pub fn clamp_f64(value: f64, min: f64, max: f64, nan: f64) -> f64 {
    if value < min {
        min
    } else if max < value {
        max
    } else if value.is_nan() {
        nan
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_within_and_outside_range() {
        assert_eq!(clamp_i32(5, 1, 10), 5);
        assert_eq!(clamp_i32(-5, 1, 10), 1);
        assert_eq!(clamp_i32(50, 1, 10), 10);
    }

    #[test]
    fn nan_uses_the_provided_fallback() {
        assert_eq!(clamp_f64(f64::NAN, 0.0, 1.0, 0.5), 0.5);
    }
}
