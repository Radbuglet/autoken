#[derive(Debug, Copy, Clone, Default)]
pub struct Pair<T> {
    pub reversed: bool,
    pub left: T,
    pub right: T,
}

impl<T> Pair<T> {
    pub fn new(left: T, right: T) -> Self {
        Self {
            reversed: false,
            left,
            right,
        }
    }

    pub fn rev(self) -> Pair<T> {
        Self {
            reversed: !self.reversed,
            left: self.right,
            right: self.left,
        }
    }

    pub fn maybe_rev(self, rev: bool) -> Pair<T> {
        if rev {
            self.rev()
        } else {
            self
        }
    }

    pub fn nat(self) -> Pair<T> {
        let rev = self.reversed;
        self.maybe_rev(rev)
    }

    pub fn map<V>(&self, left: V, right: V) -> Pair<V> {
        Pair {
            reversed: self.reversed,
            left,
            right,
        }
    }

    pub fn orders(&self) -> [Pair<&T>; 2] {
        [self.as_ref(), self.as_ref().rev()]
    }

    pub fn as_ref(&self) -> Pair<&T> {
        Pair {
            reversed: self.reversed,
            left: &self.left,
            right: &self.right,
        }
    }
}
