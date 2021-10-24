//! The `Layout` type and flexbox-like utilities for laying out components.

use smallvec::SmallVec;
use std::{
    cmp,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use super::{
    template::{ComponentDef, DynamicTemplate},
    Component,
};
use crate::terminal::{Canvas, Position, Rect, Size};

pub trait ComponentExt: Component {
    /// Creates a component definition from its `Properties`.
    fn with(properties: Self::Properties) -> Layout {
        Layout(LayoutNode::Component(DynamicTemplate(Box::new(
            ComponentDef::<Self>::new(None, properties),
        ))))
    }

    /// Creates a component definition from its `Properties`, using a custom
    /// identity specified by a key (in addition to the component's ancestors).
    ///
    /// Useful to avoid rerendering components of the same type in a container
    /// when changing the number of items in the container.
    fn with_key(key: impl Into<ComponentKey>, properties: Self::Properties) -> Layout {
        Layout(LayoutNode::Component(DynamicTemplate(Box::new(
            ComponentDef::<Self>::new(Some(key.into()), properties),
        ))))
    }

    fn item_with(flex: FlexBasis, properties: Self::Properties) -> Item {
        Item {
            flex,
            node: Layout(LayoutNode::Component(DynamicTemplate(Box::new(
                ComponentDef::<Self>::new(None, properties),
            )))),
        }
    }

    fn item_with_key(
        flex: FlexBasis,
        key: impl Into<ComponentKey>,
        properties: Self::Properties,
    ) -> Item {
        Item {
            flex,
            node: Layout(LayoutNode::Component(DynamicTemplate(Box::new(
                ComponentDef::<Self>::new(Some(key.into()), properties),
            )))),
        }
    }
}

impl<T: Component> ComponentExt for T {}

/// Wrapper type for user defined component identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComponentKey(usize);

impl From<usize> for ComponentKey {
    fn from(key: usize) -> Self {
        Self(key)
    }
}

impl From<&str> for ComponentKey {
    fn from(key: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        Self(hasher.finish() as usize)
    }
}

/// Represents a layout tree which is the main building block of a UI in Zi.
///
/// Each node in the layout tree is one
///   1. A component, any type implementing [`Component`](./Component).
///   2. A flex container that groups multiple `Layout`s, represented by
///      [`Container`](./Container).
///   3. A canvas which corresponds to the raw content in a region, represented
///      by [`Canvas`](./Canvas).
#[derive(Clone)]
pub struct Layout(pub(crate) LayoutNode);

impl Layout {
    /// Creates a new flex container with a specified direction and containing
    /// the provided items.
    ///
    /// This is a utility function that builds a container and converts it to a `Layout`.
    /// It is equivalent to calling `Container::new(direction, items).into()`.
    #[inline]
    pub fn container(direction: FlexDirection, items: impl IntoIterator<Item = Item>) -> Self {
        Container::new(direction, items).into()
    }

    /// Creates a container with column (vertical) layout.
    ///
    /// Child components are laid out from top to bottom. Pass in the children as an
    /// something that can be converted to an iterator of items, e.g. an array of
    /// items.
    ///
    /// This is a utility function that builds a container and converts it to a `Layout`.
    /// It is equivalent to calling `Container::column(items).into()`.
    #[inline]
    pub fn column(items: impl IntoIterator<Item = Item>) -> Self {
        Container::column(items).into()
    }

    /// Creates a container with reversed column (vertical) layout.
    ///
    /// Child components are laid out from bottom to top. Pass in the children as an
    /// something that can be converted to an iterator of items, e.g. an array of
    /// items.
    ///
    /// This is a utility function that builds a container and converts it to a `Layout`.
    /// It is equivalent to calling `Container::column_reverse(items).into()`.
    #[inline]
    pub fn column_reverse(items: impl IntoIterator<Item = Item>) -> Self {
        Container::column_reverse(items).into()
    }

    /// Creates a container with row (horizontal) layout.
    ///
    /// Child components are laid out from left to right. Pass in the children as an
    /// something that can be converted to an iterator of items, e.g. an array of
    /// items.
    ///
    /// This is a utility function that builds a container and converts it to a `Layout`.
    /// It is equivalent to calling `Container::row(items).into()`.
    #[inline]
    pub fn row(items: impl IntoIterator<Item = Item>) -> Self {
        Container::row(items).into()
    }

    /// Creates a container with reversed row (horizontal) layout.
    ///
    /// Child components are laid out from right to left. Pass in the children as an
    /// something that can be converted to an iterator of items, e.g. an array of
    /// items.
    ///
    /// This is a utility function that builds a container and converts it to a `Layout`.
    /// It is equivalent to calling `Container::row_reverse(items).into()`.
    #[inline]
    pub fn row_reverse(items: impl IntoIterator<Item = Item>) -> Self {
        Container::row_reverse(items).into()
    }
}

#[derive(Clone)]
pub(crate) enum LayoutNode {
    Container(Box<Container>),
    Component(DynamicTemplate),
    Canvas(Canvas),
}

impl LayoutNode {
    pub(crate) fn crawl(
        &mut self,
        frame: Rect,
        position_hash: u64,
        view_fn: &mut impl FnMut(LaidComponent),
        draw_fn: &mut impl FnMut(LaidCanvas),
    ) {
        let mut hasher = DefaultHasher::new();
        hasher.write_u64(position_hash);
        match self {
            Self::Container(container) => {
                hasher.write_u64(Self::CONTAINER_HASH);
                if container.direction.is_reversed() {
                    let frames: SmallVec<[_; ITEMS_INLINE_SIZE]> =
                        splits_iter(frame, container.direction, container.children.iter().rev())
                            .collect();
                    for (child, frame) in container.children.iter_mut().rev().zip(frames) {
                        // hasher.write_u64(Self::CONTAINER_ITEM_HASH);
                        child.node.0.crawl(frame, hasher.finish(), view_fn, draw_fn);
                    }
                } else {
                    let frames: SmallVec<[_; ITEMS_INLINE_SIZE]> =
                        splits_iter(frame, container.direction, container.children.iter())
                            .collect();
                    for (child, frame) in container.children.iter_mut().zip(frames) {
                        // hasher.write_u64(Self::CONTAINER_ITEM_HASH);
                        child.node.0.crawl(frame, hasher.finish(), view_fn, draw_fn);
                    }
                }
            }
            Self::Component(template) => {
                template.component_type_id().hash(&mut hasher);
                if let Some(key) = template.key() {
                    key.hash(&mut hasher);
                }
                view_fn(LaidComponent {
                    frame,
                    position_hash: hasher.finish(),
                    template,
                });
            }
            Self::Canvas(canvas) => {
                draw_fn(LaidCanvas { frame, canvas });
            }
        };
    }

    // Some random number to initialise the hash (0 would also do, but hopefully
    // this is less pathological if a simpler hash function is used for
    // `DefaultHasher`).
    const CONTAINER_HASH: u64 = 0x5aa2d5349a05cde8;
}

impl From<Canvas> for Layout {
    fn from(canvas: Canvas) -> Self {
        Self(LayoutNode::Canvas(canvas))
    }
}

const ITEMS_INLINE_SIZE: usize = 4;
type Items = SmallVec<[Item; ITEMS_INLINE_SIZE]>;

/// A flex container with a specified direction and items.
#[derive(Clone)]
pub struct Container {
    children: Items,
    direction: FlexDirection,
}

impl Container {
    /// Creates a new flex container with a specified direction and containing
    /// the provided items.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use zi::prelude::*;
    /// # use zi::components::text::{Text, TextProperties};
    /// # fn main() {
    /// let container = Container::new(
    ///     FlexDirection::Column,
    ///     [
    ///         Item::auto(Text::with(TextProperties::new().content("Item 1"))),
    ///         Item::auto(Text::with(TextProperties::new().content("Item 2"))),
    ///     ],
    /// );
    /// # }
    /// ```
    #[inline]
    pub fn new(direction: FlexDirection, items: impl IntoIterator<Item = Item>) -> Self {
        Self {
            children: items.into_iter().collect(),
            direction,
        }
    }

    /// Creates a new empty flex container with a specified direction.
    #[inline]
    pub fn empty(direction: FlexDirection) -> Self {
        Self {
            children: SmallVec::new(),
            direction,
        }
    }

    /// Adds an item to the end of the container.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use zi::prelude::*;
    /// # use zi::components::text::{Text, TextProperties};
    /// # fn main() {
    /// let mut container = Container::empty(FlexDirection::Row);
    /// container
    ///     .push(Item::auto(Text::with(TextProperties::new().content("Item 1"))))
    ///     .push(Item::auto(Text::with(TextProperties::new().content("Item 2"))));
    /// # }
    /// ```
    #[inline]
    pub fn push(&mut self, item: Item) -> &mut Self {
        self.children.push(item);
        self
    }

    /// Creates a container with column (vertical) layout.
    ///
    /// Child components are laid out from top to bottom. Pass in the children as an
    /// something that can be converted to an iterator of items, e.g. an array of
    /// items.
    ///
    /// This is a utility function and it is equivalent to calling
    /// `Container::new(FlexDirection::Column, items)`.
    #[inline]
    pub fn column(items: impl IntoIterator<Item = Item>) -> Self {
        Self::new(FlexDirection::Column, items)
    }

    /// Creates a container with reversed column (vertical) layout.
    ///
    /// Child components are laid out from bottom to top. Pass in the children as an
    /// something that can be converted to an iterator of items, e.g. an array of
    /// items.
    #[inline]
    pub fn column_reverse(items: impl IntoIterator<Item = Item>) -> Self {
        Self::new(FlexDirection::ColumnReverse, items)
    }

    /// Creates a container with row (horizontal) layout.
    ///
    /// Child components are laid out from left to right. Pass in the children as an
    /// something that can be converted to an iterator of items, e.g. an array of
    /// items.
    #[inline]
    pub fn row(items: impl IntoIterator<Item = Item>) -> Self {
        Self::new(FlexDirection::Row, items)
    }

    /// Creates a container with reversed row (horizontal) layout.
    ///
    /// Child components are laid out from right to left. Pass in the children as an
    /// something that can be converted to an iterator of items, e.g. an array of
    /// items.
    #[inline]
    pub fn row_reverse(items: impl IntoIterator<Item = Item>) -> Self {
        Self::new(FlexDirection::RowReverse, items)
    }
}

impl From<Container> for Layout {
    fn from(container: Container) -> Self {
        Layout(LayoutNode::Container(Box::new(container)))
    }
}

/// Represents a flex item, a layout tree nested inside a container.
///
/// An `Item` consists of a `Layout` and an associated `FlexBasis`. The latter
/// specifies how much space the layout should take along the main axis of the
/// container.
#[derive(Clone)]
pub struct Item {
    node: Layout,
    flex: FlexBasis,
}

impl Item {
    /// Creates an item that will share the available space equally with other
    /// sibling items with `FlexBasis::auto`.
    #[inline]
    pub fn auto(layout: impl Into<Layout>) -> Item {
        Item {
            node: layout.into(),
            flex: FlexBasis::Auto,
        }
    }

    /// Creates an item that will have a fixed size.
    #[inline]
    pub fn fixed<LayoutT>(size: usize) -> impl FnOnce(LayoutT) -> Item
    where
        LayoutT: Into<Layout>,
    {
        move |layout| Item {
            node: layout.into(),
            flex: FlexBasis::Fixed(size),
        }
    }
}

/// Enum to control the size of an item inside a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlexBasis {
    Auto,
    Fixed(usize),
}

/// Enum to control how items are placed in a container. It defines the main
/// axis and the direction (normal or reversed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlexDirection {
    Column,
    ColumnReverse,
    Row,
    RowReverse,
}

impl FlexDirection {
    #[inline]
    pub fn is_reversed(&self) -> bool {
        match self {
            FlexDirection::Column | FlexDirection::Row => false,
            FlexDirection::ColumnReverse | FlexDirection::RowReverse => true,
        }
    }

    #[inline]
    pub(crate) fn dimension(self, size: Size) -> usize {
        match self {
            FlexDirection::Row => size.width,
            FlexDirection::RowReverse => size.width,
            FlexDirection::Column => size.height,
            FlexDirection::ColumnReverse => size.height,
        }
    }
}

pub(crate) struct LaidComponent<'a> {
    pub frame: Rect,
    pub position_hash: u64,
    pub template: &'a mut DynamicTemplate,
}

pub(crate) struct LaidCanvas<'a> {
    pub frame: Rect,
    pub canvas: &'a Canvas,
}

#[inline]
fn splits_iter<'a>(
    frame: Rect,
    direction: FlexDirection,
    children: impl Iterator<Item = &'a Item> + Clone + 'a,
) -> impl Iterator<Item = Rect> + 'a {
    let total_size = direction.dimension(frame.size);

    // Compute how much space is available for stretched components
    let (stretched_budget, num_stretched_children, total_fixed_size) = {
        let mut stretched_budget = total_size;
        let mut num_stretched_children = 0;
        let mut total_fixed_size = 0;
        for child in children.clone() {
            match child.flex {
                FlexBasis::Auto => {
                    num_stretched_children += 1;
                }
                FlexBasis::Fixed(size) => {
                    stretched_budget = stretched_budget.saturating_sub(size);
                    total_fixed_size += size;
                }
            }
        }
        (stretched_budget, num_stretched_children, total_fixed_size)
    };

    // Divvy up the space equaly between stretched components.
    let stretched_size = if num_stretched_children > 0 {
        stretched_budget / num_stretched_children
    } else {
        0
    };
    let mut remainder =
        total_size.saturating_sub(num_stretched_children * stretched_size + total_fixed_size);
    let mut remaining_size = total_size;

    children
        .map(move |child| match child.flex {
            FlexBasis::Auto => {
                let offset = total_size - remaining_size;
                let size = if remainder > 0 {
                    remainder -= 1;
                    stretched_size + 1
                } else {
                    stretched_size
                };
                remaining_size -= size;
                (offset, size)
            }
            FlexBasis::Fixed(size) => {
                let offset = total_size - remaining_size;
                let size = cmp::min(remaining_size, size);
                remaining_size -= size;
                (offset, size)
            }
        })
        .map(move |(offset, size)| match direction {
            FlexDirection::Row | FlexDirection::RowReverse => Rect::new(
                Position::new(frame.origin.x + offset, frame.origin.y),
                Size::new(size, frame.size.height),
            ),
            FlexDirection::Column | FlexDirection::ColumnReverse => Rect::new(
                Position::new(frame.origin.x, frame.origin.y + offset),
                Size::new(frame.size.width, size),
            ),
        })
}
