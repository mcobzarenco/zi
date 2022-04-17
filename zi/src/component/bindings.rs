use smallvec::{smallvec, SmallVec};
use std::{
    any::{Any, TypeId},
    borrow::Cow,
    collections::hash_map::HashMap,
    fmt,
    marker::PhantomData,
};

use super::{Component, DynamicMessage};
use crate::terminal::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CommandId(usize);

#[derive(Clone, Debug, PartialEq)]
pub enum NamedBindingQuery {
    Match(Cow<'static, str>),
    PrefixOf(SmallVec<[Cow<'static, str>; 4]>),
}

impl NamedBindingQuery {
    pub fn new(keymap: &Keymap, query: &BindingQuery) -> Self {
        match query {
            BindingQuery::Match(command_id) => Self::Match(keymap.names[command_id.0].clone()),
            BindingQuery::PrefixOf(commands) => Self::PrefixOf(
                commands
                    .iter()
                    .map(|command_id| keymap.names[command_id.0].clone())
                    .collect(),
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BindingQuery {
    Match(CommandId),
    PrefixOf(SmallVec<[CommandId; 4]>),
}

impl BindingQuery {
    pub fn matches(&self) -> Option<CommandId> {
        match self {
            Self::Match(command_id) => Some(*command_id),
            _ => None,
        }
    }

    pub fn prefix_of(&self) -> Option<&[CommandId]> {
        match self {
            Self::PrefixOf(commands) => Some(commands),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Keymap {
    names: Vec<Cow<'static, str>>,
    keymap: HashMap<KeyPattern, BindingQuery>,
}

impl Keymap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(&self, command_id: &CommandId) -> &str {
        &self.names[command_id.0]
    }

    pub fn is_empty(&self) -> bool {
        self.keymap.is_empty()
    }

    pub fn add(
        &mut self,
        name: impl Into<Cow<'static, str>>,
        pattern: impl Into<KeyPattern>,
    ) -> CommandId {
        let command_id = self.add_command(name).0;
        self.bind_command(command_id, pattern);
        command_id
    }

    pub fn add_command(&mut self, name: impl Into<Cow<'static, str>>) -> (CommandId, bool) {
        let name = name.into();
        let (command_id, is_new_command) = self
            .names
            .iter()
            .enumerate()
            .find(|(_index, existing)| **existing == name)
            .map(|(index, _)| (CommandId(index), false))
            .unwrap_or_else(|| (CommandId(self.names.len()), true));
        if is_new_command {
            self.names.push(name);
        }
        (command_id, is_new_command)
    }

    pub fn bind_command(&mut self, command_id: CommandId, pattern: impl Into<KeyPattern>) {
        let name = &self.names[command_id.0];
        let pattern = pattern.into();

        // Add `BindingQuery::PrefixOf` entries for all prefixes of the key sequence
        if let Some(keys) = pattern.keys() {
            for prefix_len in 0..keys.len() {
                let prefix = KeyPattern::Keys(keys.iter().copied().take(prefix_len).collect());
                self.keymap
                    .entry(prefix.clone())
                    .and_modify(|entry| match entry {
                        BindingQuery::Match(other_command_id) => panic_on_overlapping_key_bindings(
                            &pattern,
                            name,
                            &prefix,
                            &self.names[other_command_id.0],
                        ),
                        BindingQuery::PrefixOf(prefix_of) => {
                            prefix_of.push(command_id);
                        }
                    })
                    .or_insert_with(|| BindingQuery::PrefixOf(smallvec![command_id]));
            }
        }

        // Add a `BindingQuery::Match` for the full key sequence
        self.keymap
            .entry(pattern.clone())
            .and_modify(|entry| match entry {
                BindingQuery::Match(other_command_id) => panic_on_overlapping_key_bindings(
                    &pattern,
                    name,
                    &pattern,
                    &self.names[other_command_id.0],
                ),
                BindingQuery::PrefixOf(prefix_of) => panic_on_overlapping_key_bindings(
                    &pattern,
                    name,
                    &pattern,
                    &self.names[prefix_of[0].0],
                ),
            })
            .or_insert_with(|| BindingQuery::Match(command_id));
    }

    pub fn check_sequence(&self, keys: &[Key]) -> Option<&BindingQuery> {
        let pattern: KeyPattern = keys.iter().copied().into();
        self.keymap
            .get(&pattern)
            .or_else(|| match keys {
                &[Key::Char(_)] => self.keymap.get(&KeyPattern::AnyCharacter),
                _ => None,
            })
            .or_else(|| match keys {
                &[_, key] | &[key] => self.keymap.get(&KeyPattern::EndsWith([key])),
                _ => None,
            })
    }
}

#[allow(clippy::type_complexity)]
struct DynamicCommandFn(Box<dyn Fn(&dyn Any, &[Key]) -> Option<DynamicMessage>>);

impl fmt::Debug for DynamicCommandFn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CommandFn@{:?})", &self.0 as *const _)
    }
}

#[derive(Debug)]
pub(crate) struct DynamicBindings {
    keymap: Keymap,
    commands: Vec<DynamicCommandFn>,
    focused: bool,
    notify: bool,
    type_id: TypeId,
}

impl DynamicBindings {
    pub fn new<ComponentT: Component>() -> Self {
        Self {
            keymap: Keymap::new(),
            commands: Vec::new(),
            focused: false,
            notify: false,
            type_id: TypeId::of::<ComponentT>(),
        }
    }

    #[inline]
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    #[inline]
    pub fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
    }

    #[inline]
    pub fn focused(&self) -> bool {
        self.focused
    }

    #[inline]
    pub fn set_notify(&mut self, notify: bool) {
        self.notify = notify;
    }

    #[inline]
    pub fn notify(&self) -> bool {
        self.notify
    }

    pub fn add<ComponentT: Component, const VARIANT: usize>(
        &mut self,
        name: impl Into<Cow<'static, str>>,
        keys: impl Into<KeyPattern>,
        command_fn: impl CommandFn<ComponentT, VARIANT> + 'static,
    ) -> CommandId {
        let command_id = self.add_command(name, command_fn);
        self.bind_command(command_id, keys);
        command_id
    }

    pub fn add_command<ComponentT: Component, const VARIANT: usize>(
        &mut self,
        name: impl Into<Cow<'static, str>>,
        command_fn: impl CommandFn<ComponentT, VARIANT> + 'static,
    ) -> CommandId {
        assert_eq!(self.type_id, TypeId::of::<ComponentT>());

        let (command_id, is_new_command) = self.keymap.add_command(name);
        let dyn_command_fn = DynamicCommandFn(Box::new(move |erased: &dyn Any, keys: &[Key]| {
            let component = erased
                .downcast_ref()
                .expect("Incorrect `Component` type when downcasting");
            command_fn
                .call(component, keys)
                .map(|message| DynamicMessage(Box::new(message)))
        }));
        if is_new_command {
            self.commands.push(dyn_command_fn);
        } else {
            self.commands[command_id.0] = dyn_command_fn;
        }

        command_id
    }

    pub fn bind_command(&mut self, command_id: CommandId, keys: impl Into<KeyPattern>) {
        self.keymap.bind_command(command_id, keys);
    }

    pub fn execute_command<ComponentT: Component>(
        &self,
        component: &ComponentT,
        id: CommandId,
        keys: &[Key],
    ) -> Option<DynamicMessage> {
        assert_eq!(self.type_id, TypeId::of::<ComponentT>());

        (self.commands[id.0].0)(component, keys)
    }

    pub fn typed<ComponentT: Component>(
        &mut self,
        callback: impl FnOnce(&mut Bindings<ComponentT>),
    ) {
        assert_eq!(self.type_id, TypeId::of::<ComponentT>());

        let mut bindings = Self::new::<ComponentT>();
        std::mem::swap(self, &mut bindings);
        let mut typed = Bindings::<ComponentT>::new(bindings);
        callback(&mut typed);
        std::mem::swap(self, &mut typed.bindings);
    }
}

#[derive(Debug)]
pub struct Bindings<ComponentT> {
    bindings: DynamicBindings,
    _component: PhantomData<fn() -> ComponentT>,
}

impl<ComponentT: Component> Bindings<ComponentT> {
    fn new(bindings: DynamicBindings) -> Self {
        Self {
            bindings,
            _component: PhantomData,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bindings.keymap.is_empty()
    }

    #[inline]
    pub fn set_focus(&mut self, focused: bool) {
        self.bindings.set_focus(focused)
    }

    #[inline]
    pub fn focused(&self) -> bool {
        self.bindings.focused()
    }

    #[inline]
    pub fn set_notify(&mut self, notify: bool) {
        self.bindings.set_notify(notify)
    }

    #[inline]
    pub fn notify(&self) -> bool {
        self.bindings.notify()
    }

    #[inline]
    pub fn add<const VARIANT: usize>(
        &mut self,
        name: impl Into<Cow<'static, str>>,
        keys: impl Into<KeyPattern>,
        command_fn: impl CommandFn<ComponentT, VARIANT> + 'static,
    ) {
        self.bindings.add(name, keys, command_fn);
    }

    #[inline]
    pub fn command<const VARIANT: usize>(
        &mut self,
        name: impl Into<Cow<'static, str>>,
        command_fn: impl CommandFn<ComponentT, VARIANT> + 'static,
    ) -> BindingBuilder<ComponentT> {
        let command_id = self.bindings.add_command(name, command_fn);
        BindingBuilder {
            wrapped: self,
            command_id,
        }
    }
}

#[derive(Debug)]
pub struct BindingBuilder<'a, ComponentT> {
    wrapped: &'a mut Bindings<ComponentT>,
    command_id: CommandId,
}

impl<ComponentT: Component> BindingBuilder<'_, ComponentT> {
    pub fn with(self, keys: impl Into<KeyPattern>) -> Self {
        self.wrapped.bindings.bind_command(self.command_id, keys);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyPattern {
    AnyCharacter,
    EndsWith([Key; 1]),
    Keys(SmallVec<[Key; 8]>),
}

impl KeyPattern {
    fn keys(&self) -> Option<&[Key]> {
        match self {
            Self::AnyCharacter => None,
            Self::EndsWith(key) => Some(key.as_slice()),
            Self::Keys(keys) => Some(keys.as_slice()),
        }
    }
}

impl<IterT: IntoIterator<Item = Key>> From<IterT> for KeyPattern {
    fn from(keys: IterT) -> Self {
        Self::Keys(keys.into_iter().collect())
    }
}

impl std::fmt::Display for KeyPattern {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        match self {
            Self::AnyCharacter => {
                write!(formatter, "Char(*)")
            }
            Self::Keys(keys) => KeySequenceSlice(keys.as_slice()).fmt(formatter),
            Self::EndsWith(keys) => KeySequenceSlice(keys.as_slice()).fmt(formatter),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnyCharacter;

impl From<AnyCharacter> for KeyPattern {
    fn from(_: AnyCharacter) -> Self {
        Self::AnyCharacter
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EndsWith(pub Key);

impl From<EndsWith> for KeyPattern {
    fn from(ends_with: EndsWith) -> Self {
        Self::EndsWith([ends_with.0])
    }
}

pub trait CommandFn<ComponentT: Component, const VARIANT: usize> {
    fn call(&self, component: &ComponentT, keys: &[Key]) -> Option<ComponentT::Message>;
}

// Specializations for callbacks that take either a component or slice with keys
// and return an option
impl<ComponentT, FnT> CommandFn<ComponentT, 0> for FnT
where
    ComponentT: Component,
    FnT: Fn(&ComponentT, &[Key]) -> Option<ComponentT::Message> + 'static,
{
    fn call(&self, component: &ComponentT, keys: &[Key]) -> Option<ComponentT::Message> {
        (self)(component, keys)
    }
}

impl<ComponentT, FnT> CommandFn<ComponentT, 1> for FnT
where
    ComponentT: Component,
    FnT: Fn(&ComponentT) -> Option<ComponentT::Message> + 'static,
{
    #[inline]
    fn call(&self, component: &ComponentT, _keys: &[Key]) -> Option<ComponentT::Message> {
        (self)(component)
    }
}

impl<ComponentT, FnT> CommandFn<ComponentT, 2> for FnT
where
    ComponentT: Component,
    FnT: Fn(&[Key]) -> Option<ComponentT::Message> + 'static,
{
    #[inline]
    fn call(&self, _component: &ComponentT, keys: &[Key]) -> Option<ComponentT::Message> {
        (self)(keys)
    }
}

// Specializations for callbacks that take a component and optionally a slice with keys
impl<ComponentT, FnT> CommandFn<ComponentT, 3> for FnT
where
    ComponentT: Component,
    FnT: Fn(&ComponentT, &[Key]) + 'static,
{
    #[inline]
    fn call(&self, component: &ComponentT, keys: &[Key]) -> Option<ComponentT::Message> {
        (self)(component, keys);
        None
    }
}

impl<ComponentT, FnT> CommandFn<ComponentT, 4> for FnT
where
    ComponentT: Component,
    FnT: Fn(&ComponentT) + 'static,
{
    #[inline]
    fn call(&self, component: &ComponentT, _keys: &[Key]) -> Option<ComponentT::Message> {
        (self)(component);
        None
    }
}

// Specialization for callbacks that take no parameters and return a message
impl<ComponentT, FnT> CommandFn<ComponentT, 5> for FnT
where
    ComponentT: Component,
    FnT: Fn() -> ComponentT::Message + 'static,
{
    #[inline]
    fn call(&self, _component: &ComponentT, _keys: &[Key]) -> Option<ComponentT::Message> {
        Some((self)())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySequenceSlice<'a>(&'a [Key]);

impl<'a> From<&'a [Key]> for KeySequenceSlice<'a> {
    fn from(keys: &'a [Key]) -> Self {
        Self(keys)
    }
}

impl<'a> std::fmt::Display for KeySequenceSlice<'a> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        for (index, key) in self.0.iter().enumerate() {
            match key {
                Key::Char(' ') => write!(formatter, "SPC")?,
                Key::Char('\n') => write!(formatter, "RET")?,
                Key::Char('\t') => write!(formatter, "TAB")?,
                Key::Char(char) => write!(formatter, "{}", char)?,
                Key::Ctrl(char) => write!(formatter, "C-{}", char)?,
                Key::Alt(char) => write!(formatter, "A-{}", char)?,
                Key::F(number) => write!(formatter, "F{}", number)?,
                Key::Esc => write!(formatter, "ESC")?,
                key => write!(formatter, "{:?}", key)?,
            }
            if index < self.0.len().saturating_sub(1) {
                write!(formatter, " ")?;
            }
        }
        Ok(())
    }
}

fn panic_on_overlapping_key_bindings(
    new_pattern: &KeyPattern,
    new_name: &str,
    existing_pattern: &KeyPattern,
    existing_name: &str,
) -> ! {
    panic!(
        "Binding `{}` for `{}` is ambiguous as it overlaps with binding `{}` for command `{}`",
        new_pattern, new_name, existing_pattern, existing_name,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use smallvec::smallvec;
    use std::{cell::RefCell, rc::Rc};

    struct Empty;

    impl Component for Empty {
        type Message = ();
        type Properties = ();

        fn create(_: Self::Properties, _: Rect, _: ComponentLink<Self>) -> Self {
            Self
        }

        fn view(&self) -> Layout {
            Canvas::new(Size::new(10, 10)).into()
        }
    }

    #[test]
    fn keymap_alternative_binding_for_same_command() {
        let mut keymap = Keymap::new();
        let right_id = keymap.add("right", [Key::Right]);
        let left_id = keymap.add("left", [Key::Left]);
        assert_ne!(left_id, right_id);
        let alternate_left_id = keymap.add("left", [Key::Ctrl('b')]);
        assert_eq!(left_id, alternate_left_id);
    }

    #[test]
    fn controller_one_command_end_to_end() {
        let called = Rc::new(RefCell::new(false));

        // Create a controller with one registered command
        let mut controller = DynamicBindings::new::<Empty>();
        let test_command_id = controller.add("test-command", [Key::Ctrl('x'), Key::Ctrl('f')], {
            let called = Rc::clone(&called);
            move |_: &Empty| {
                *called.borrow_mut() = true;
                None
            }
        });

        // Check no key sequence is a prefix of test-command
        assert_eq!(
            controller.keymap().check_sequence(&[]),
            Some(&BindingQuery::PrefixOf(smallvec![test_command_id]))
        );
        // Check C-x is a prefix of test-command
        assert_eq!(
            controller.keymap().check_sequence(&[Key::Ctrl('x')]),
            Some(&BindingQuery::PrefixOf(smallvec![test_command_id]))
        );
        // Check C-x C-f is a match for test-command
        assert_eq!(
            controller
                .keymap()
                .check_sequence(&[Key::Ctrl('x'), Key::Ctrl('f')]),
            Some(&BindingQuery::Match(test_command_id))
        );

        // Check C-f doesn't match any command
        assert_eq!(controller.keymap().check_sequence(&[Key::Ctrl('f')]), None);
        // Check C-x C-x doesn't match any command
        assert_eq!(
            controller
                .keymap()
                .check_sequence(&[Key::Ctrl('x'), Key::Ctrl('x')]),
            None
        );

        controller.execute_command(&Empty, test_command_id, &[]);
        assert!(*called.borrow(), "set-controller wasn't called");
    }
}
