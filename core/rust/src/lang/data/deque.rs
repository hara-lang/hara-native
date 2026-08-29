//! Persistent deque backed by a count-measured finger tree.

use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    HashType, IAssoc, IColl, IConj, ICons, ICount, IDisplay, IEmpty, IEquality, IHash, ILookup,
    IMetadata, INth, IObjType, IPeekFirst, IPeekLast, IPersistent, IPopFirst, IPopLast, IPushFirst,
    IPushLast, MetaType, ObjType,
};
use std::rc::Rc;

#[derive(Debug, Clone)]
enum Item<E> {
    Leaf(E),
    Branch {
        measure: usize,
        children: Vec<Rc<Item<E>>>,
    },
}

impl<E: Clone> Item<E> {
    fn leaf(value: E) -> Rc<Self> {
        Rc::new(Self::Leaf(value))
    }
    fn branch(children: Vec<Rc<Self>>) -> Rc<Self> {
        debug_assert!((2..=3).contains(&children.len()));
        Rc::new(Self::Branch {
            measure: children.iter().map(|child| child.measure()).sum(),
            children,
        })
    }
    fn measure(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Branch { measure, .. } => *measure,
        }
    }
    fn get(&self, mut index: usize) -> Option<&E> {
        match self {
            Self::Leaf(value) => (index == 0).then_some(value),
            Self::Branch { children, .. } => {
                for child in children {
                    if index < child.measure() {
                        return child.get(index);
                    }
                    index -= child.measure();
                }
                None
            }
        }
    }
    fn replace(&self, mut index: usize, value: E) -> Option<Rc<Self>> {
        match self {
            Self::Leaf(_) => (index == 0).then(|| Self::leaf(value)),
            Self::Branch { children, .. } => {
                for (position, child) in children.iter().enumerate() {
                    if index < child.measure() {
                        let mut replaced = children.clone();
                        replaced[position] = child.replace(index, value)?;
                        return Some(Self::branch(replaced));
                    }
                    index -= child.measure();
                }
                None
            }
        }
    }
    fn collect<'a>(&'a self, output: &mut Vec<&'a E>) {
        match self {
            Self::Leaf(value) => output.push(value),
            Self::Branch { children, .. } => {
                for child in children {
                    child.collect(output);
                }
            }
        }
    }
    fn children(&self) -> Vec<Rc<Self>> {
        match self {
            Self::Branch { children, .. } => children.clone(),
            Self::Leaf(_) => unreachable!("finger-tree middle item must be a branch"),
        }
    }
}

#[derive(Debug, Clone, Default)]
enum FingerTree<E> {
    #[default]
    Empty,
    Single(Rc<Item<E>>),
    Deep {
        measure: usize,
        prefix: Vec<Rc<Item<E>>>,
        middle: Rc<FingerTree<E>>,
        suffix: Vec<Rc<Item<E>>>,
    },
}

impl<E: Clone> FingerTree<E> {
    fn measure(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Single(item) => item.measure(),
            Self::Deep { measure, .. } => *measure,
        }
    }
    fn deep(prefix: Vec<Rc<Item<E>>>, middle: Rc<Self>, suffix: Vec<Rc<Item<E>>>) -> Self {
        debug_assert!((1..=4).contains(&prefix.len()));
        debug_assert!((1..=4).contains(&suffix.len()));
        let measure = prefix.iter().map(|item| item.measure()).sum::<usize>()
            + middle.measure()
            + suffix.iter().map(|item| item.measure()).sum::<usize>();
        Self::Deep {
            measure,
            prefix,
            middle,
            suffix,
        }
    }
    fn push_first_item(&self, item: Rc<Item<E>>) -> Self {
        match self {
            Self::Empty => Self::Single(item),
            Self::Single(existing) => {
                Self::deep(vec![item], Rc::new(Self::Empty), vec![existing.clone()])
            }
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } if prefix.len() < 4 => {
                let mut next = Vec::with_capacity(prefix.len() + 1);
                next.push(item);
                next.extend(prefix.iter().cloned());
                Self::deep(next, middle.clone(), suffix.clone())
            }
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } => {
                let node = Item::branch(prefix[1..].to_vec());
                Self::deep(
                    vec![item, prefix[0].clone()],
                    Rc::new(middle.push_first_item(node)),
                    suffix.clone(),
                )
            }
        }
    }
    fn push_last_item(&self, item: Rc<Item<E>>) -> Self {
        match self {
            Self::Empty => Self::Single(item),
            Self::Single(existing) => {
                Self::deep(vec![existing.clone()], Rc::new(Self::Empty), vec![item])
            }
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } if suffix.len() < 4 => {
                let mut next = suffix.clone();
                next.push(item);
                Self::deep(prefix.clone(), middle.clone(), next)
            }
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } => {
                let node = Item::branch(suffix[..3].to_vec());
                Self::deep(
                    prefix.clone(),
                    Rc::new(middle.push_last_item(node)),
                    vec![suffix[3].clone(), item],
                )
            }
        }
    }
    fn from_items(items: Vec<Rc<Item<E>>>) -> Self {
        items
            .into_iter()
            .fold(Self::Empty, |tree, item| tree.push_last_item(item))
    }
    fn pop_first_item(&self) -> Option<(Rc<Item<E>>, Self)> {
        match self {
            Self::Empty => None,
            Self::Single(item) => Some((item.clone(), Self::Empty)),
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } if prefix.len() > 1 => Some((
                prefix[0].clone(),
                Self::deep(prefix[1..].to_vec(), middle.clone(), suffix.clone()),
            )),
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } => {
                if let Some((node, next_middle)) = middle.pop_first_item() {
                    Some((
                        prefix[0].clone(),
                        Self::deep(node.children(), Rc::new(next_middle), suffix.clone()),
                    ))
                } else {
                    Some((prefix[0].clone(), Self::from_items(suffix.clone())))
                }
            }
        }
    }
    fn pop_last_item(&self) -> Option<(Rc<Item<E>>, Self)> {
        match self {
            Self::Empty => None,
            Self::Single(item) => Some((item.clone(), Self::Empty)),
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } if suffix.len() > 1 => {
                let last = suffix.len() - 1;
                Some((
                    suffix[last].clone(),
                    Self::deep(prefix.clone(), middle.clone(), suffix[..last].to_vec()),
                ))
            }
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } => {
                if let Some((node, next_middle)) = middle.pop_last_item() {
                    Some((
                        suffix[0].clone(),
                        Self::deep(prefix.clone(), Rc::new(next_middle), node.children()),
                    ))
                } else {
                    Some((suffix[0].clone(), Self::from_items(prefix.clone())))
                }
            }
        }
    }
    fn get(&self, mut index: usize) -> Option<&E> {
        match self {
            Self::Empty => None,
            Self::Single(item) => item.get(index),
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } => {
                for item in prefix {
                    if index < item.measure() {
                        return item.get(index);
                    }
                    index -= item.measure();
                }
                if index < middle.measure() {
                    return middle.get(index);
                }
                index -= middle.measure();
                for item in suffix {
                    if index < item.measure() {
                        return item.get(index);
                    }
                    index -= item.measure();
                }
                None
            }
        }
    }
    fn replace(&self, mut index: usize, value: E) -> Option<Self> {
        match self {
            Self::Empty => None,
            Self::Single(item) => Some(Self::Single(item.replace(index, value)?)),
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } => {
                for (position, item) in prefix.iter().enumerate() {
                    if index < item.measure() {
                        let mut next = prefix.clone();
                        next[position] = item.replace(index, value)?;
                        return Some(Self::deep(next, middle.clone(), suffix.clone()));
                    }
                    index -= item.measure();
                }
                if index < middle.measure() {
                    return Some(Self::deep(
                        prefix.clone(),
                        Rc::new(middle.replace(index, value)?),
                        suffix.clone(),
                    ));
                }
                index -= middle.measure();
                for (position, item) in suffix.iter().enumerate() {
                    if index < item.measure() {
                        let mut next = suffix.clone();
                        next[position] = item.replace(index, value)?;
                        return Some(Self::deep(prefix.clone(), middle.clone(), next));
                    }
                    index -= item.measure();
                }
                None
            }
        }
    }
    fn collect<'a>(&'a self, output: &mut Vec<&'a E>) {
        match self {
            Self::Empty => {}
            Self::Single(item) => item.collect(output),
            Self::Deep {
                prefix,
                middle,
                suffix,
                ..
            } => {
                for item in prefix {
                    item.collect(output);
                }
                middle.collect(output);
                for item in suffix {
                    item.collect(output);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Standard<E> {
    metadata: Option<Rc<crate::lang::data::Metadata>>,
    tree: FingerTree<E>,
}

impl<E: Clone> Default for Standard<E> {
    fn default() -> Self {
        Self {
            metadata: None,
            tree: FingerTree::Empty,
        }
    }
}
impl<E: Clone> Standard<E> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.tree.measure()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, index: usize) -> Option<&E> {
        self.tree.get(index)
    }
    pub fn push_first(&self, value: E) -> Self {
        Self {
            metadata: self.metadata.clone(),
            tree: self.tree.push_first_item(Item::leaf(value)),
        }
    }
    pub fn push_last(&self, value: E) -> Self {
        Self {
            metadata: self.metadata.clone(),
            tree: self.tree.push_last_item(Item::leaf(value)),
        }
    }
    pub fn pop_first_value(&self) -> Self {
        self.tree
            .pop_first_item()
            .map(|(_, tree)| Self {
                metadata: self.metadata.clone(),
                tree,
            })
            .unwrap_or_else(|| self.clone())
    }
    pub fn pop_last_value(&self) -> Self {
        self.tree
            .pop_last_item()
            .map(|(_, tree)| Self {
                metadata: self.metadata.clone(),
                tree,
            })
            .unwrap_or_else(|| self.clone())
    }
    pub fn assoc_value(&self, index: usize, value: E) -> Option<Self> {
        Some(Self {
            metadata: self.metadata.clone(),
            tree: self.tree.replace(index, value)?,
        })
    }
    pub fn iter(&self) -> std::vec::IntoIter<&E> {
        let mut values = Vec::with_capacity(self.len());
        self.tree.collect(&mut values);
        values.into_iter()
    }
}
impl<E: Clone> FromIterator<E> for Standard<E> {
    fn from_iter<T: IntoIterator<Item = E>>(it: T) -> Self {
        it.into_iter().fold(Self::new(), |d, v| d.push_last(v))
    }
}
impl<E: Clone> IntoIterator for Standard<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<E: Clone + PartialEq> PartialEq for Standard<E> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<E: Clone> ICount for Standard<E> {
    fn count(&self) -> usize {
        self.len()
    }
}
impl<E: Clone> INth<E> for Standard<E> {
    fn nth(&self, index: usize) -> Option<&E> {
        self.get(index)
    }
}
impl<E: Clone> ILookup<usize, E> for Standard<E> {
    type Keys = std::ops::Range<usize>;
    type Values = std::vec::IntoIter<E>;
    fn keys(&self) -> Self::Keys {
        0..self.len()
    }
    fn vals(&self) -> Self::Values {
        self.iter().cloned().collect::<Vec<_>>().into_iter()
    }
}
impl<E: Clone> crate::lang::protocol::IFind<usize> for Standard<E> {
    type Output = (usize, E);
    fn find(&self, key: &usize) -> Option<Self::Output> {
        self.get(*key).cloned().map(|v| (*key, v))
    }
}
impl<E: Clone> IAssoc<usize, E> for Standard<E> {
    type Output = Self;
    fn assoc(&self, k: usize, v: E) -> Self {
        self.assoc_value(k, v).unwrap_or_else(|| self.clone())
    }
}
impl<E: Clone> IPeekFirst<E> for Standard<E> {
    fn peek_first(&self) -> Option<E> {
        self.get(0).cloned()
    }
}
impl<E: Clone> IPeekLast<E> for Standard<E> {
    fn peek_last(&self) -> Option<E> {
        self.len().checked_sub(1).and_then(|i| self.get(i)).cloned()
    }
}
impl<E: Clone> IPushFirst<E> for Standard<E> {
    type Output = Self;
    fn push_first(&self, v: E) -> Self {
        Standard::push_first(self, v)
    }
}
impl<E: Clone> IPushLast<E> for Standard<E> {
    type Output = Self;
    fn push_last(&self, v: E) -> Self {
        Standard::push_last(self, v)
    }
}
impl<E: Clone> IPopFirst for Standard<E> {
    type Output = Self;
    fn pop_first(&self) -> Self {
        self.pop_first_value()
    }
}
impl<E: Clone> IPopLast for Standard<E> {
    type Output = Self;
    fn pop_last(&self) -> Self {
        self.pop_last_value()
    }
}
impl<E: Clone> ICons<E> for Standard<E> {
    type Output = Self;
    fn cons(&self, v: E) -> Self {
        self.push_first(v)
    }
}
impl<E: Clone> IConj<E> for Standard<E> {
    type Output = Self;
    fn conj(&self, v: E) -> Self {
        self.push_last(v)
    }
}
impl<E: Clone> IEmpty for Standard<E> {
    type Output = Self;
    fn empty(&self) -> Self {
        Self::new().with_meta(self.metadata.clone())
    }
}
impl<E: Clone> IMetadata for Standard<E> {
    type Metadata = Rc<crate::lang::data::Metadata>;
    fn meta(&self) -> Option<&Self::Metadata> {
        self.metadata.as_ref()
    }
    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        Self {
            metadata,
            tree: self.tree.clone(),
        }
    }
    fn metatype(&self) -> MetaType {
        MetaType::Object
    }
}
impl<E: Clone> IPersistent for Standard<E> {}
impl<E: Clone + PartialEq> IEquality for Standard<E> {
    fn equality(&self, other: &Self) -> bool {
        self == other
    }
}
impl<E: Clone + std::fmt::Debug> IDisplay for Standard<E> {
    fn display(&self) -> String {
        format!(
            "[{}]",
            self.iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
impl<E: Clone + std::hash::Hash + JavaHash> IHash for Standard<E> {
    fn hash_calc(&self, t: HashType) -> u64 {
        crate::lang::hash::compose_ordered("SEQUENTIAL", self.iter().map(|v| v.java_hash(t))) as u64
    }
}
impl<E: Clone + std::fmt::Debug> IObjType for Standard<E> {
    fn obj_type(&self) -> ObjType {
        ObjType::Sequential
    }
}
impl<E> IColl<E> for Standard<E>
where
    E: Clone + PartialEq + std::hash::Hash + JavaHash + std::fmt::Debug,
{
    fn start_string(&self) -> &'static str {
        "["
    }
    fn end_string(&self) -> &'static str {
        "]"
    }
}

#[cfg(test)]
mod tests {
    use super::Standard;
    #[test]
    fn finger_tree_deque_matches_two_ended_model() {
        let mut deque = Standard::new();
        let mut model = std::collections::VecDeque::new();
        for i in 0..2000 {
            if i % 3 == 0 {
                deque = deque.push_first(i);
                model.push_front(i)
            } else {
                deque = deque.push_last(i);
                model.push_back(i)
            }
        }
        assert_eq!(
            deque.iter().copied().collect::<Vec<_>>(),
            model.iter().copied().collect::<Vec<_>>()
        );
        for i in 0..1500 {
            if i % 2 == 0 {
                deque = deque.pop_first_value();
                model.pop_front();
            } else {
                deque = deque.pop_last_value();
                model.pop_back();
            }
        }
        assert_eq!(
            deque.iter().copied().collect::<Vec<_>>(),
            model.iter().copied().collect::<Vec<_>>()
        );
        assert_eq!(deque.len(), model.len());
    }
    #[test]
    fn persistence_and_index_replacement_are_preserved() {
        let original: Standard<_> = (0..100).collect();
        let changed = original.assoc_value(50, 999).unwrap();
        assert_eq!(original.get(50), Some(&50));
        assert_eq!(changed.get(50), Some(&999));
        assert_eq!(changed.get(99), Some(&99));
    }
}
