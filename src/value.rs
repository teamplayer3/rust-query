use std::{
    cell::OnceCell,
    ops::Deref,
    sync::atomic::{AtomicU64, Ordering},
};

use elsa::FrozenVec;
use sea_query::{Expr, Iden, IntoColumnRef, SimpleExpr};

use crate::{
    ast::{Joins, MyTable},
    Builder, HasId,
};

pub trait Value<'t>: Sized {
    type Typ: MyIdenT;
    fn build_expr(&self) -> SimpleExpr;

    fn add<T: Value<'t>>(self, rhs: T) -> MyAdd<Self, T> {
        MyAdd(self, rhs)
    }

    fn lt(self, rhs: i32) -> MyLt<Self> {
        MyLt(self, rhs)
    }

    fn eq<T: Value<'t>>(self, rhs: T) -> MyEq<Self, T> {
        MyEq(self, rhs)
    }

    fn not(self) -> MyNot<Self> {
        MyNot(self)
    }
}

pub trait ValueOpt<'t>: Value<'t, Typ = Option<Self::Inner>> {
    type Inner: MyIdenT;

    fn unwrap_or<T: Value<'t, Typ = Self::Inner>>(self, rhs: T) -> UnwrapOr<Self, T> {
        UnwrapOr(self, rhs)
    }
}

impl<'t, T: Value<'t>> Value<'t> for &'_ T {
    type Typ = T::Typ;

    fn build_expr(&self) -> SimpleExpr {
        T::build_expr(self)
    }
}

impl<'t, T: MyIdenT> Value<'t> for Db<'t, T> {
    type Typ = T;
    fn build_expr(&self) -> SimpleExpr {
        Expr::col(self.field).into()
    }
}

impl<'t, T: MyIdenT> ValueOpt<'t> for Db<'t, Option<T>> {
    type Inner = T;
}

#[derive(Clone, Copy)]
pub struct MyAdd<A, B>(A, B);

impl<'t, A: Value<'t>, B: Value<'t>> Value<'t> for MyAdd<A, B> {
    type Typ = A::Typ;
    fn build_expr(&self) -> SimpleExpr {
        self.0.build_expr().add(self.1.build_expr())
    }
}

#[derive(Clone, Copy)]
pub struct MyNot<T>(T);

impl<'t, T: Value<'t>> Value<'t> for MyNot<T> {
    type Typ = T::Typ;
    fn build_expr(&self) -> SimpleExpr {
        self.0.build_expr().not()
    }
}

#[derive(Clone, Copy)]
pub struct MyLt<A>(A, i32);

impl<'t, A: Value<'t>> Value<'t> for MyLt<A> {
    type Typ = bool;
    fn build_expr(&self) -> SimpleExpr {
        Expr::expr(self.0.build_expr()).lt(self.1)
    }
}

#[derive(Clone, Copy)]
pub struct MyEq<A, B>(A, B);

impl<'t, A: Value<'t>, B: Value<'t>> Value<'t> for MyEq<A, B> {
    type Typ = bool;
    fn build_expr(&self) -> SimpleExpr {
        self.0.build_expr().eq(self.1.build_expr())
    }
}

#[derive(Clone, Copy)]
pub struct Const<T>(T);

impl<T> Const<T> {
    pub fn new<V: ToOwned<Owned = T> + ?Sized>(val: &V) -> Self {
        Self(val.to_owned())
    }
}

impl<'t, T: MyIdenT> Value<'t> for Const<T>
where
    T: Into<sea_query::value::Value> + Clone,
{
    type Typ = T;
    fn build_expr(&self) -> SimpleExpr {
        SimpleExpr::from(self.0.clone())
    }
}

#[derive(Clone, Copy)]
pub struct UnwrapOr<A, B>(pub(crate) A, pub(crate) B);

impl<'t, T: MyIdenT, A: ValueOpt<'t, Inner = T>, B: Value<'t, Typ = T>> Value<'t>
    for UnwrapOr<A, B>
{
    type Typ = T;
    fn build_expr(&self) -> SimpleExpr {
        Expr::expr(self.0.build_expr()).if_null(self.1.build_expr())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FieldAlias {
    pub table: MyAlias,
    pub col: Field,
}

impl IntoColumnRef for FieldAlias {
    fn into_column_ref(self) -> sea_query::ColumnRef {
        (self.table, self.col).into_column_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Field {
    U64(MyAlias),
    Str(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct MyAlias {
    name: u64,
}

impl sea_query::Iden for Field {
    fn unquoted(&self, s: &mut dyn std::fmt::Write) {
        match self {
            Field::U64(alias) => alias.unquoted(s),
            Field::Str(name) => write!(s, "{}", name).unwrap(),
        }
    }
}

impl MyAlias {
    pub fn new() -> Self {
        static IDEN_NUM: AtomicU64 = AtomicU64::new(0);
        let next = IDEN_NUM.fetch_add(1, Ordering::Relaxed);
        Self { name: next }
    }
}

impl sea_query::Iden for MyAlias {
    fn unquoted(&self, s: &mut dyn std::fmt::Write) {
        write!(s, "_{}", self.name).unwrap()
    }
}

pub(super) trait MyTableT<'t> {
    fn unwrap(joined: &'t FrozenVec<Box<(Field, MyTable)>>) -> Self;
}

impl<'t, T: HasId> MyTableT<'t> for FkInfo<'t, T> {
    fn unwrap(joined: &'t FrozenVec<Box<(Field, MyTable)>>) -> Self {
        FkInfo {
            joined,
            inner: OnceCell::new(),
        }
    }
}

impl<'t> MyTableT<'t> for ValueInfo {
    fn unwrap(_joined: &'t FrozenVec<Box<(Field, MyTable)>>) -> Self {
        ValueInfo {}
    }
}

pub(super) struct FkInfo<'t, T: HasId> {
    pub joined: &'t FrozenVec<Box<(Field, MyTable)>>, // the table that we join onto
    pub inner: OnceCell<Box<T::Dummy<'t>>>,
}

impl<'t, T: HasId> FkInfo<'t, T> {
    pub(crate) fn joined(
        joined: &'t FrozenVec<Box<(Field, MyTable)>>,
        field: FieldAlias,
    ) -> Db<'t, T> {
        Db {
            info: FkInfo {
                joined,
                // prevent unnecessary join
                inner: OnceCell::from(Box::new(T::build(Builder::new_full(joined, field.table)))),
            },
            field,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ValueInfo {}

pub(super) trait MyIdenT: Sized {
    type Info<'t>: MyTableT<'t>;
    fn iden_any(joins: &Joins, col: Field) -> Db<'_, Self> {
        let field = FieldAlias {
            table: joins.table,
            col,
        };
        Self::iden_full(&joins.joined, field)
    }
    fn iden_full(joined: &FrozenVec<Box<(Field, MyTable)>>, field: FieldAlias) -> Db<'_, Self> {
        Db {
            info: Self::Info::unwrap(joined),
            field,
        }
    }
}

impl<T: HasId> MyIdenT for T {
    type Info<'t> = FkInfo<'t, T>;
}

impl MyIdenT for i64 {
    type Info<'t> = ValueInfo;
}

impl MyIdenT for f64 {
    type Info<'t> = ValueInfo;
}

impl MyIdenT for bool {
    type Info<'t> = ValueInfo;
}

impl MyIdenT for String {
    type Info<'t> = ValueInfo;
}

impl<T: MyIdenT> MyIdenT for Option<T> {
    type Info<'t> = ValueInfo;
}

// invariant in `'t` because of the associated type
pub struct Db<'t, T: MyIdenT> {
    pub(super) info: T::Info<'t>,
    pub(super) field: FieldAlias,
}

impl<'t, T: MyIdenT> Clone for Db<'t, T>
where
    T::Info<'t>: Clone,
{
    fn clone(&self) -> Self {
        Db {
            info: self.info.clone(),
            field: self.field,
        }
    }
}
impl<'t, T: MyIdenT> Copy for Db<'t, T> where T::Info<'t>: Copy {}

impl<'a, T: HasId> Db<'a, T> {
    pub fn id(&self) -> Db<'a, i64> {
        Db {
            info: ValueInfo {},
            field: self.field,
        }
    }
}

impl<'a, T: HasId> Deref for Db<'a, T> {
    type Target = T::Dummy<'a>;

    fn deref(&self) -> &Self::Target {
        self.info.inner.get_or_init(|| {
            let joined = self.info.joined;
            let name = self.field.col;
            let table = if let Some(item) = joined.iter().find(|item| item.0 == name) {
                &item.1
            } else {
                let table = MyTable {
                    name: T::NAME,
                    id: T::ID,
                    joins: Joins {
                        table: MyAlias::new(),
                        joined: FrozenVec::new(),
                    },
                };
                &joined.push_get(Box::new((name, table))).1
            };

            Box::new(T::build(Builder::new(&table.joins)))
        })
    }
}

pub(crate) struct RawAlias(pub(crate) String);

impl Iden for RawAlias {
    fn unquoted(&self, s: &mut dyn std::fmt::Write) {
        write!(s, "{}", self.0).unwrap()
    }
    fn prepare(&self, s: &mut dyn std::fmt::Write, _q: sea_query::Quote) {
        self.unquoted(s)
    }
}
