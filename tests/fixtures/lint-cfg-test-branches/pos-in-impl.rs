pub struct Probe {
    hook: i32,
}

impl Probe {
    pub fn tick(&self) {
        #[cfg(test)]
        self.hook += 1;
    }
}
