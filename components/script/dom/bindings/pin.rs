/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! Pins, also known as immovable roots.

use dom::bindings::reflector::DomObject;
use dom::bindings::root::Dom;
use dom::bindings::trace::JSTraceable;
use js::jsapi::JSTracer;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::mem;
use std::ops::Drop;

#[allow(unrooted_must_root)]
#[allow_unrooted_interior]
pub struct Pin<'this, T>
where
    T: JSTraceable + 'static,
{
    marker: PhantomData<&'this ()>,
    cell: Option<PinCell<T>>,
}

impl<'this, T> Pin<'this, T>
where
    T: JSTraceable,
{
    pub unsafe fn new() -> Self {
        Self { marker: PhantomData, cell: None }
    }

    pub fn pin<U>(&'this mut self, traced: U) -> &'this T
    where
        T: UntracedFrom<U>,
    {
        unsafe {
            self.cell = Some(PinCell::new(T::untraced_from(traced)));
            self.cell.as_mut().unwrap().pin()
        }
    }
}

pub trait UntracedDefault: 'static {
    unsafe fn untraced_default() -> Self;
}

macro_rules! impl_untraceddefault_as_default {
    (for<$($param:ident),*> $ty:ty) => {
        impl<$($param),*> UntracedDefault for $ty
        where
            $($param: 'static),*
        {
            #[inline]
            unsafe fn untraced_default() -> Self {
                Default::default()
            }
        }
    };
}

impl_untraceddefault_as_default!(for<T> Vec<T>);

pub trait UntracedFrom<T>: 'static {
    unsafe fn untraced_from(traced: T) -> Self;
}

impl<'a, T> UntracedFrom<&'a mut T> for T
where
    T: UntracedDefault + 'static,
{
    #[inline]
    unsafe fn untraced_from(traced: &'a mut T) -> Self {
        mem::replace(traced, T::untraced_default())
    }
}

impl<'a, T, U> UntracedFrom<&'a [U]> for Vec<T>
where
    T: UntracedFrom<&'a U> + 'static,
{
    #[inline]
    unsafe fn untraced_from(traced: &'a [U]) -> Self {
        traced.iter().map(|x| T::untraced_from(x)).collect()
    }
}

impl<'a, T> UntracedFrom<&'a T> for Dom<T>
where
    T: DomObject + 'static,
{
    #[allow(unrooted_must_root)]
    #[inline]
    unsafe fn untraced_from(traced: &'a T) -> Self {
        Dom::from_ref(traced)
    }
}

impl<'a, T> UntracedFrom<&'a Dom<T>> for Dom<T>
where
    T: DomObject + 'static,
{
    #[allow(unrooted_must_root)]
    #[inline]
    unsafe fn untraced_from(traced: &'a Dom<T>) -> Self {
        Dom::from_ref(&**traced)
    }
}

pub unsafe fn initialize() {
    PINNED_TRACEABLES.with(|cell| {
        let mut cell = cell.borrow_mut();
        assert!(cell.is_none(), "pin list has already been initialized");
        *cell = Some(None);
    });
}

pub unsafe fn trace(tracer: *mut JSTracer) {
    trace!("tracing stack-rooted pins");
    PINNED_TRACEABLES.with(|ref cell| {
        let cell = cell.borrow();
        let mut head = cell.unwrap();
        while let Some(current) = head {
            (*current).value.trace(tracer);
            head = (*current).prev;
        }
    });
}

thread_local! {
    static PINNED_TRACEABLES: RefCell<Option<Option<*const PinCell<JSTraceable>>>> =
        Default::default();
}

struct PinCell<T>
where
    T: JSTraceable + ?Sized + 'static,
{
    prev: Option<*const PinCell<JSTraceable>>,
    value: T,
}

impl<T> PinCell<T>
where
    T: JSTraceable + 'static,
{
    unsafe fn new(untraced: T) -> Self {
        Self { prev: None, value: untraced }
    }

    unsafe fn pin<'pin>(&'pin mut self) -> &'pin T {
        let this = self as &PinCell<JSTraceable> as *const _;
        PINNED_TRACEABLES.with(|cell| {
            self.prev = mem::replace(
                cell.borrow_mut().as_mut().unwrap(),
                Some(this),
            );
        });
        &self.value
    }
}

impl<T> Drop for PinCell<T>
where
    T: JSTraceable + ?Sized + 'static,
{
    fn drop(&mut self) {
        PINNED_TRACEABLES.with(|cell| {
            *cell.borrow_mut().as_mut().unwrap() = self.prev;
        });
    }
}
