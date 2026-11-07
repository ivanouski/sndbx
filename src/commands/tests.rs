use super::*;

#[test]
fn cli_is_valid() {
    command().debug_assert();
}

#[test]
fn line_counts() {
    assert_eq!(line_count("10"), Ok(10));
    assert_eq!(line_count("+3"), Ok(3));
    assert_eq!(line_count("-0"), Ok(0));
    assert!(line_count("-1").is_err());
    assert!(line_count("x").is_err());
    assert!(line_count("0x10").is_err());
}
