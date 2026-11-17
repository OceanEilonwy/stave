//! A list that always holds at least one element.

use serde::{Serialize, Serializer};

/// A list with at least one element: where an empty list would be invalid
/// (the kinds a part's boards are joined by, the boards of a part), the
/// type says so. It serializes as a plain list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmpty<T> {
    head: T,
    tail: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// The list, or `None` when `items` is empty.
    pub fn new(items: Vec<T>) -> Option<NonEmpty<T>> {
        let mut items = items.into_iter();
        let head = items.next()?;
        Some(NonEmpty {
            head,
            tail: items.collect(),
        })
    }

    /// A list of one element.
    pub fn one(item: T) -> NonEmpty<T> {
        NonEmpty {
            head: item,
            tail: Vec::new(),
        }
    }

    /// The first element.
    pub fn first(&self) -> &T {
        &self.head
    }

    /// The elements in order.
    pub fn iter(&self) -> std::iter::Chain<std::iter::Once<&T>, std::slice::Iter<'_, T>> {
        std::iter::once(&self.head).chain(self.tail.iter())
    }

    /// How many elements there are (at least one).
    pub fn count(&self) -> usize {
        self.tail.len().saturating_add(1)
    }

    /// The element at `index` (0 is the first).
    pub fn get(&self, index: usize) -> Option<&T> {
        match index.checked_sub(1) {
            None => Some(&self.head),
            Some(rest) => self.tail.get(rest),
        }
    }
}

impl<T: Serialize> Serialize for NonEmpty<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_list_is_refused() {
        assert_eq!(NonEmpty::<u8>::new(Vec::new()), None);
        let list = NonEmpty::new(vec![3, 4]).unwrap();
        assert_eq!(*list.first(), 3);
        assert_eq!(list.count(), 2);
        assert_eq!(list.get(1), Some(&4));
        assert_eq!(list.get(2), None);
        assert_eq!(list.iter().copied().collect::<Vec<_>>(), [3, 4]);
        assert_eq!(NonEmpty::one(7).count(), 1);
    }
}
