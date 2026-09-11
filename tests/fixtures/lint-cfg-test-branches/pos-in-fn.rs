pub fn run(flag: bool) -> i32 {
    #[cfg(test)]
    {
        return 1;
    }
    if flag {
        2
    } else {
        0
    }
}
