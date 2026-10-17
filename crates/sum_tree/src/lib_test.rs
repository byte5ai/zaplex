use super::*;
use std::ops::Add;

#[test]
fn test_cursor() {
    // Empty tree
    let tree = SumTree::<u8>::new();
    let mut cursor = tree.cursor::<Count, Sum>();
    assert_eq!(
        cursor.slice(&Count(0), SeekBias::Right).items(),
        Vec::<u8>::new()
    );
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    // Single-element tree
    let mut tree = SumTree::<u8>::new();
    tree.extend(vec![1]);
    let mut cursor = tree.cursor::<Count, Sum>();
    assert_eq!(
        cursor.slice(&Count(0), SeekBias::Right).items(),
        Vec::<u8>::new()
    );
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    cursor.next();
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    let mut cursor = tree.cursor::<Count, Sum>();
    assert_eq!(cursor.slice(&Count(1), SeekBias::Right).items(), [1]);
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    cursor.seek(&Count(0), SeekBias::Right);
    assert_eq!(
        cursor
            .slice(&tree.extent::<Count>(), SeekBias::Right)
            .items(),
        [1]
    );
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    assert!(!cursor.seek(&Count(2), SeekBias::Left));

    cursor.seek(&Count(0), SeekBias::Left);
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    // Left bias should position it at item 1
    cursor.seek_clamped(&Count(2), SeekBias::Left);
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    // Right bias should position it past item 1
    cursor.seek_clamped(&Count(2), SeekBias::Right);
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    // Multiple-element tree
    let mut tree = SumTree::new();
    tree.extend(vec![1, 2, 3, 4, 5, 6]);
    let mut cursor = tree.cursor::<Count, Sum>();

    assert_eq!(cursor.slice(&Count(2), SeekBias::Right).items(), [1, 2]);
    assert_eq!(cursor.item(), Some(&3));
    assert_eq!(cursor.prev_item(), Some(&2));
    assert_eq!(cursor.start(), &Sum(3));

    cursor.next();
    assert_eq!(cursor.item(), Some(&4));
    assert_eq!(cursor.prev_item(), Some(&3));
    assert_eq!(cursor.start(), &Sum(6));

    cursor.next();
    assert_eq!(cursor.item(), Some(&5));
    assert_eq!(cursor.prev_item(), Some(&4));
    assert_eq!(cursor.start(), &Sum(10));

    cursor.next();
    assert_eq!(cursor.item(), Some(&6));
    assert_eq!(cursor.prev_item(), Some(&5));
    assert_eq!(cursor.start(), &Sum(15));

    cursor.next();
    cursor.next();
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(21));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&6));
    assert_eq!(cursor.prev_item(), Some(&5));
    assert_eq!(cursor.start(), &Sum(15));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&5));
    assert_eq!(cursor.prev_item(), Some(&4));
    assert_eq!(cursor.start(), &Sum(10));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&4));
    assert_eq!(cursor.prev_item(), Some(&3));
    assert_eq!(cursor.start(), &Sum(6));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&3));
    assert_eq!(cursor.prev_item(), Some(&2));
    assert_eq!(cursor.start(), &Sum(3));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&2));
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    cursor.prev();
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    cursor.next();
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    let mut cursor = tree.cursor::<Count, Sum>();
    assert_eq!(
        cursor
            .slice(&tree.extent::<Count>(), SeekBias::Right)
            .items(),
        tree.items()
    );
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(21));

    cursor.seek(&Count(3), SeekBias::Right);
    assert_eq!(
        cursor
            .slice(&tree.extent::<Count>(), SeekBias::Right)
            .items(),
        [4, 5, 6]
    );
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(21));

    // Seeking can bias left or right
    cursor.seek(&Count(1), SeekBias::Left);
    assert_eq!(cursor.item(), Some(&1));
    cursor.seek(&Count(1), SeekBias::Right);
    assert_eq!(cursor.item(), Some(&2));

    // Slicing without resetting starts from where the cursor is parked at.
    cursor.seek(&Count(1), SeekBias::Right);
    assert_eq!(cursor.slice(&Count(3), SeekBias::Right).items(), vec![2, 3]);
    assert_eq!(cursor.slice(&Count(6), SeekBias::Left).items(), vec![4, 5]);
    assert_eq!(cursor.slice(&Count(6), SeekBias::Right).items(), vec![6]);

    // Seek with a max
    let mut cursor = tree.cursor::<Count, Sum>();
    cursor.seek_clamped(&Count(1), SeekBias::Right);
    assert_eq!(cursor.slice(&Count(3), SeekBias::Right).items(), vec![2, 3]);

    // Seek max past the end
    cursor.seek_clamped(&Count(8), SeekBias::Right);
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(21));
    assert_eq!(
        cursor.rev().collect::<Vec<&u8>>(),
        vec![&6, &5, &4, &3, &2, &1]
    );

    // Seek max past the end with a left bias
    let mut cursor = tree.cursor::<Count, Sum>();
    cursor.seek(&Count(1), SeekBias::Right);
    cursor.seek_clamped(&Count(8), SeekBias::Left);
    assert_eq!(cursor.item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(15));
    assert_eq!(
        cursor.rev().collect::<Vec<&u8>>(),
        vec![&6, &5, &4, &3, &2, &1]
    );

    // Seek backwards - this works because seeking multiple times
    // resets the cursor state.
    let mut cursor = tree.cursor::<Count, Sum>();
    cursor.seek(&Count(3), SeekBias::Right);
    assert_eq!(cursor.item(), Some(&4));
    cursor.seek(&Count(1), SeekBias::Right);
    assert_eq!(cursor.item(), Some(&2));
}

#[derive(Clone, Default, Debug)]
pub struct IntegersSummary {
    count: Count,
    sum: Sum,
    contains_even: bool,
}

#[derive(Ord, PartialOrd, Default, Eq, PartialEq, Clone, Debug)]
struct Count(usize);

#[derive(Ord, PartialOrd, Default, Eq, PartialEq, Clone, Debug)]
struct Sum(usize);

impl Item for u8 {
    type Summary = IntegersSummary;

    fn summary(&self) -> Self::Summary {
        IntegersSummary {
            count: Count(1),
            sum: Sum(*self as usize),
            contains_even: (*self & 1) == 0,
        }
    }
}

impl AddAssign<&Self> for IntegersSummary {
    fn add_assign(&mut self, other: &Self) {
        self.count.0 += &other.count.0;
        self.sum.0 += &other.sum.0;
        self.contains_even |= other.contains_even;
    }
}

impl Dimension<'_, IntegersSummary> for Count {
    fn add_summary(&mut self, summary: &IntegersSummary) {
        self.0 += summary.count.0;
    }
}

impl Dimension<'_, IntegersSummary> for Sum {
    fn add_summary(&mut self, summary: &IntegersSummary) {
        self.0 += summary.sum.0;
    }
}

impl Add<&Self> for Sum {
    type Output = Self;

    fn add(mut self, other: &Self) -> Self {
        self.0 += other.0;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Height(f32);

impl Eq for Height {}

impl PartialOrd for Height {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Height {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[derive(Clone, Debug, Default)]
struct MeasuredSummary {
    height: Height,
    count: Count,
}

impl AddAssign<&Self> for MeasuredSummary {
    fn add_assign(&mut self, other: &Self) {
        self.height.0 += other.height.0;
        self.count.0 += other.count.0;
    }
}

impl Dimension<'_, MeasuredSummary> for Height {
    fn add_summary(&mut self, summary: &MeasuredSummary) {
        self.0 += summary.height.0;
    }
}

impl Dimension<'_, MeasuredSummary> for Count {
    fn add_summary(&mut self, summary: &MeasuredSummary) {
        self.0 += summary.count.0;
    }
}
