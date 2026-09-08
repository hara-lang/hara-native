//! Host-local, single-assignment lazy values. Dropping the last reference
//! releases an unrealized thunk or its cached outcome; realization never resets.
use std::cell::RefCell;
use std::rc::Rc;

enum State<V> {
    Pending(Box<dyn FnOnce() -> Result<V, String>>),
    Running,
    Ready(Result<V, String>),
}

#[derive(Clone)]
pub struct Delay<V> {
    state: Rc<RefCell<State<V>>>,
}

impl<V> Delay<V> {
    pub fn new(thunk: impl FnOnce() -> Result<V, String> + 'static) -> Self {
        Self { state: Rc::new(RefCell::new(State::Pending(Box::new(thunk)))) }
    }

    pub fn is_realized(&self) -> bool {
        matches!(*self.state.borrow(), State::Ready(_))
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }

    pub fn identity_address(&self) -> usize {
        Rc::as_ptr(&self.state) as usize
    }
}

impl<V: Clone> Delay<V> {
    pub fn deref_value(&self) -> Result<V, String> {
        let thunk = {
            let mut state = self.state.borrow_mut();
            match &*state {
                State::Ready(result) => return result.clone(),
                State::Running => return Err("delay realization is recursive".into()),
                State::Pending(_) => {}
            }
            match std::mem::replace(&mut *state, State::Running) {
                State::Pending(thunk) => thunk,
                _ => unreachable!(),
            }
        };
        // Do not hold the RefCell borrow while invoking user code.
        let result = thunk();
        *self.state.borrow_mut() = State::Ready(result.clone());
        result
    }
}

impl<V> std::fmt::Debug for Delay<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delay").field("realized", &self.is_realized()).finish()
    }
}
