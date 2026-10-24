pub fn positive(value: i32) -> bool {
    value > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_one() {
        assert!(positive(1));
    }

    #[test]
    fn positive_two() {
        assert!(positive(2));
    }

    #[test]
    fn zero() {
        assert!(!positive(0));
    }

    #[test]
    fn negative() {
        assert!(!positive(-1));
    }

    #[test]
    #[ignore = "Not part of the observed suite"]
    fn ignored() {
        panic!("must not run");
    }
}
