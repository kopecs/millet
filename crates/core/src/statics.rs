//! Statics.
//!
//! With help from [this article][1].
//!
//! [1]: http://dev.stephendiehl.com/fun/006_hindley_milner.html

#![allow(unused)]

use crate::ast::{Cases, Dec, Exp, Label, Long, Pat, StrDec, TopDec, Ty as AstTy};
use crate::intern::{StrRef, StrStore};
use crate::loc::{Loc, Located};
use maplit::{hashmap, hashset};
use std::collections::{HashMap, HashSet};
use std::convert::TryInto as _;
use std::fmt;

pub fn get(top_decs: &[Located<TopDec<StrRef>>]) -> Result<()> {
  let (mut bs, mut st) = std_lib();
  for top_dec in top_decs {
    bs = ck_top_dec(bs, &mut st, top_dec)?;
  }
  let subst = st.solve()?;
  bs.apply(&subst);
  assert!(bs.free_ty_vars().is_empty());
  Ok(())
}

pub enum StaticsError {
  Undefined(Item, StrRef),
  Redefined(StrRef),
  DuplicateLabel(Label),
  Circularity(TyVar, Ty),
  HeadMismatch(Ty, Ty),
  MissingLabel(Label),
  ValAsPat,
  WrongNumTyArgs(usize, usize),
  NonVarInAs(StrRef),
  ForbiddenBinding(StrRef),
  NoSuitableOverload,
  TyNameEscape,
  Todo,
}

impl StaticsError {
  pub fn message(&self, store: &StrStore) -> String {
    match self {
      Self::Undefined(item, id) => format!("undefined {} identifier: {}", item, store.get(*id)),
      Self::Redefined(id) => format!("redefined identifier: {}", store.get(*id)),
      Self::DuplicateLabel(lab) => format!("duplicate label: {}", show_lab(store, *lab)),
      Self::Circularity(ty_var, ty) => {
        format!("circularity: {:?} in {}", ty_var, show_ty(store, &ty))
      }
      Self::HeadMismatch(lhs, rhs) => format!(
        "mismatched types: {} vs {}",
        show_ty(store, &lhs),
        show_ty(store, &rhs)
      ),
      Self::MissingLabel(lab) => format!("type is missing label {}", show_lab(store, *lab)),
      Self::ValAsPat => "value binding used as pattern".to_owned(),
      Self::WrongNumTyArgs(want, got) => format!(
        "wrong number of type arguments: expected {}, found {}",
        want, got
      ),
      Self::NonVarInAs(id) => format!(
        "pattern to left of `as` is not a variable: {}",
        store.get(*id)
      ),
      Self::ForbiddenBinding(id) => format!("forbidden identifier in binding: {}", store.get(*id)),
      Self::NoSuitableOverload => "no suitable overload found".to_owned(),
      Self::TyNameEscape => "expression causes a type name to escape its scope".to_owned(),
      Self::Todo => "unimplemented language construct".to_owned(),
    }
  }
}

fn show_lab(store: &StrStore, lab: Label) -> String {
  match lab {
    Label::Vid(id) => store.get(id).to_owned(),
    Label::Num(n) => format!("{}", n),
  }
}

fn show_ty(store: &StrStore, ty: &Ty) -> String {
  let mut buf = String::new();
  show_ty_impl(&mut buf, store, ty);
  buf
}

fn show_ty_impl(buf: &mut String, store: &StrStore, ty: &Ty) {
  match ty {
    Ty::Var(tv) => buf.push_str(&format!("{:?}", tv)),
    Ty::Record(rows) => {
      buf.push_str("{ ");
      let mut rows = rows.iter();
      if let Some((lab, ty)) = rows.next() {
        show_row(buf, store, *lab, ty);
      }
      for (lab, ty) in rows {
        buf.push_str(", ");
        show_row(buf, store, *lab, ty);
      }
      buf.push_str(" }");
    }
    Ty::Arrow(lhs, rhs) => {
      buf.push_str("(");
      show_ty_impl(buf, store, lhs);
      buf.push_str(") -> (");
      show_ty_impl(buf, store, rhs);
      buf.push_str(")");
    }
    Ty::Ctor(args, sym) => {
      if args.is_empty() {
        buf.push_str(store.get(sym.name()));
        return;
      }
      buf.push_str("(");
      let mut args = args.iter();
      if let Some(arg) = args.next() {
        show_ty_impl(buf, store, arg);
      }
      for arg in args {
        buf.push_str(", ");
        show_ty_impl(buf, store, arg);
      }
      buf.push_str(") ");
      buf.push_str(store.get(sym.name()));
    }
  }
}

fn show_row(buf: &mut String, store: &StrStore, lab: Label, ty: &Ty) {
  buf.push_str(&show_lab(store, lab));
  buf.push_str(" : ");
  show_ty_impl(buf, store, ty);
}

pub type Result<T> = std::result::Result<T, Located<StaticsError>>;

pub enum Item {
  Value,
  Type,
  Structure,
  Signature,
  Functor,
}

impl fmt::Display for Item {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Value => write!(f, "value"),
      Self::Type => write!(f, "type"),
      Self::Structure => write!(f, "structure"),
      Self::Signature => write!(f, "signature"),
      Self::Functor => write!(f, "functor"),
    }
  }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Sym {
  name: StrRef,
  id: Option<Located<usize>>,
}

impl Sym {
  const fn base(name: StrRef) -> Self {
    Self { name, id: None }
  }

  pub const fn name(&self) -> StrRef {
    self.name
  }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct TyVar {
  id: usize,
  equality: bool,
}

impl fmt::Debug for TyVar {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "'")?;
    if self.equality {
      write!(f, "'")?;
    }
    write!(f, "t{}", self.id)
  }
}

#[derive(Clone, Default)]
struct Subst {
  inner: HashMap<TyVar, Ty>,
}

impl Subst {
  fn insert(&mut self, tv: TyVar, ty: Ty) {
    for (_, other) in self.inner.iter_mut() {
      other.apply(&Subst {
        inner: hashmap![tv => ty.clone()],
      });
    }
    assert!(self.inner.insert(tv, ty).is_none());
  }
}

trait Substitutable {
  fn apply(&mut self, subst: &Subst);
  fn free_ty_vars(&self) -> TyVarSet;
}

#[derive(Debug, Clone)]
pub enum Ty {
  /// TyVar
  Var(TyVar),
  /// RowType. Use a Vec to preserve source order. TODO row polymorphism?
  Record(Vec<(Label, Ty)>),
  /// FunType
  Arrow(Box<Ty>, Box<Ty>),
  /// ConsType
  Ctor(Vec<Ty>, Sym),
}

impl Ty {
  const fn base(name: StrRef) -> Self {
    Self::Ctor(Vec::new(), Sym::base(name))
  }

  fn list(elem: Self) -> Self {
    Self::Ctor(vec![elem], Sym::base(StrRef::LIST))
  }

  fn ref_(elem: Self) -> Self {
    Self::Ctor(vec![elem], Sym::base(StrRef::REF))
  }

  fn ty_names(&self) -> TyNameSet {
    match self {
      Self::Var(_) => TyNameSet::new(),
      Self::Record(rows) => rows.iter().flat_map(|(_, ty)| ty.ty_names()).collect(),
      Self::Arrow(arg, res) => arg.ty_names().into_iter().chain(res.ty_names()).collect(),
      Self::Ctor(args, sym) => std::iter::once(sym.name())
        .chain(args.iter().flat_map(Self::ty_names))
        .collect(),
    }
  }

  const CHAR: Self = Self::base(StrRef::CHAR);
  const EXN: Self = Self::base(StrRef::EXN);
  const BOOL: Self = Self::base(StrRef::BOOL);
  const STRING: Self = Self::base(StrRef::STRING);
  const WORD: Self = Self::base(StrRef::WORD);
  const INT: Self = Self::base(StrRef::INT);
  const REAL: Self = Self::base(StrRef::REAL);
  const ORDER: Self = Self::base(StrRef::ORDER);
}

impl Substitutable for Ty {
  fn apply(&mut self, subst: &Subst) {
    match self {
      Self::Var(tv) => match subst.inner.get(tv) {
        None => {}
        Some(ty) => *self = ty.clone(),
      },
      Self::Record(rows) => {
        for (_, ty) in rows {
          ty.apply(subst);
        }
      }
      Self::Arrow(lhs, rhs) => {
        lhs.apply(subst);
        rhs.apply(subst);
      }
      Self::Ctor(args, _) => {
        for arg in args {
          arg.apply(subst);
        }
      }
    }
  }

  fn free_ty_vars(&self) -> TyVarSet {
    match self {
      Self::Var(tv) => hashset![*tv],
      Self::Record(rows) => rows.iter().flat_map(|(_, ty)| ty.free_ty_vars()).collect(),
      Self::Arrow(lhs, rhs) => lhs
        .free_ty_vars()
        .union(&rhs.free_ty_vars())
        .copied()
        .collect(),
      Self::Ctor(args, _) => args.iter().flat_map(Self::free_ty_vars).collect(),
    }
  }
}

#[derive(Clone)]
struct TyScheme {
  ty_vars: Vec<TyVar>,
  ty: Ty,
  /// See `instantiate` and `Constraints#solve`.
  overload: Option<Vec<StrRef>>,
}

impl TyScheme {
  fn mono(ty: Ty) -> Self {
    Self {
      ty_vars: Vec::new(),
      ty,
      overload: None,
    }
  }
}

impl Substitutable for TyScheme {
  fn apply(&mut self, subst: &Subst) {
    let mut subst = subst.clone();
    for tv in self.ty_vars.iter() {
      subst.inner.remove(tv);
    }
    self.ty.apply(&subst)
  }

  fn free_ty_vars(&self) -> TyVarSet {
    self
      .ty
      .free_ty_vars()
      .difference(&self.ty_vars.iter().copied().collect())
      .copied()
      .collect()
  }
}

type TyFcn = TyScheme;

#[derive(Clone)]
struct TyInfo {
  ty_fcn: TyFcn,
  val_env: ValEnv,
}

type StrEnv = HashMap<StrRef, Env>;

#[derive(Clone, Default)]
struct TyEnv {
  inner: HashMap<StrRef, TyInfo>,
}

impl Substitutable for TyEnv {
  fn apply(&mut self, subst: &Subst) {
    for (_, ty_info) in self.inner.iter_mut() {
      ty_info.ty_fcn.apply(subst);
    }
  }

  fn free_ty_vars(&self) -> TyVarSet {
    self
      .inner
      .iter()
      .flat_map(|(_, ty_info)| ty_info.ty_fcn.free_ty_vars())
      .collect()
  }
}

#[derive(Clone, PartialEq, Eq)]
enum IdStatus {
  Ctor,
  Exn,
  Val,
}

#[derive(Clone)]
struct ValInfo {
  ty_scheme: TyScheme,
  id_status: IdStatus,
}

impl ValInfo {
  fn ctor(ty_scheme: TyScheme) -> Self {
    Self {
      ty_scheme,
      id_status: IdStatus::Ctor,
    }
  }

  fn exn() -> Self {
    Self {
      ty_scheme: TyScheme::mono(Ty::EXN),
      id_status: IdStatus::Exn,
    }
  }

  fn val(ty_scheme: TyScheme) -> Self {
    Self {
      ty_scheme,
      id_status: IdStatus::Ctor,
    }
  }
}

type ValEnv = HashMap<StrRef, ValInfo>;

#[derive(Clone, Default)]
struct Env {
  str_env: StrEnv,
  ty_env: TyEnv,
  val_env: ValEnv,
}

impl Env {
  /// `other` overwrites `self`.
  fn extend(&mut self, other: Self) {
    for (name, env) in other.str_env {
      self.str_env.insert(name, env);
    }
    for (name, ty_info) in other.ty_env.inner {
      self.ty_env.inner.insert(name, ty_info);
    }
    for (name, val_info) in other.val_env {
      self.val_env.insert(name, val_info);
    }
  }

  fn ty_names(&self) -> TyNameSet {
    self
      .str_env
      .iter()
      .flat_map(|(_, env)| env.ty_names())
      .chain(self.ty_env.inner.keys().copied())
      .collect()
  }
}

impl From<ValEnv> for Env {
  fn from(val_env: ValEnv) -> Self {
    Self {
      str_env: StrEnv::new(),
      ty_env: TyEnv::default(),
      val_env,
    }
  }
}

impl From<TyEnv> for Env {
  fn from(ty_env: TyEnv) -> Self {
    Self {
      str_env: StrEnv::new(),
      ty_env,
      val_env: ValEnv::new(),
    }
  }
}

impl Substitutable for Env {
  fn apply(&mut self, subst: &Subst) {
    for (_, env) in self.str_env.iter_mut() {
      env.apply(subst);
    }
    self.ty_env.apply(subst);
    for (_, val_info) in self.val_env.iter_mut() {
      val_info.ty_scheme.apply(subst);
    }
  }

  fn free_ty_vars(&self) -> TyVarSet {
    self
      .str_env
      .iter()
      .flat_map(|(_, env)| env.free_ty_vars())
      .chain(self.ty_env.free_ty_vars())
      .chain(
        self
          .val_env
          .iter()
          .flat_map(|(_, vi)| vi.ty_scheme.free_ty_vars()),
      )
      .collect()
  }
}

type TyNameSet = HashSet<StrRef>;

type TyVarSet = HashSet<TyVar>;

#[derive(Clone)]
struct Cx {
  ty_names: TyNameSet,
  ty_vars: TyVarSet,
  env: Env,
}

impl Cx {
  // this is the o-plus operation defined in the Definition, which extends a context by an
  // environment and that environment's type name set.
  fn o_plus(&mut self, env: Env) {
    let ty_names = env.ty_names();
    self.env.extend(env);
    self.ty_names.extend(ty_names);
  }
}

struct Sig {
  ty_names: TyNameSet,
  env: Env,
}

struct FunSig {
  ty_names: TyNameSet,
  env: Env,
  sig: Sig,
}

type SigEnv = HashMap<StrRef, Sig>;

type FunEnv = HashMap<StrRef, FunSig>;

struct Basis {
  ty_names: TyNameSet,
  fun_env: FunEnv,
  sig_env: SigEnv,
  env: Env,
}

impl Substitutable for Basis {
  fn apply(&mut self, subst: &Subst) {
    for (_, fun_sig) in self.fun_env.iter_mut() {
      fun_sig.env.apply(subst);
      fun_sig.sig.env.apply(subst);
    }
    for (_, sig) in self.sig_env.iter_mut() {
      sig.env.apply(subst);
    }
    self.env.apply(subst);
  }

  fn free_ty_vars(&self) -> TyVarSet {
    self
      .fun_env
      .iter()
      .flat_map(|(_, fun_sig)| {
        fun_sig
          .env
          .free_ty_vars()
          .into_iter()
          .chain(fun_sig.sig.env.free_ty_vars())
      })
      .chain(
        self
          .sig_env
          .iter()
          .flat_map(|(_, sig)| sig.env.free_ty_vars()),
      )
      .chain(self.env.free_ty_vars())
      .collect()
  }
}

#[derive(Default)]
struct State {
  next_ty_var: usize,
  next_sym: usize,
  overload: Vec<(Loc, TyVar, Vec<StrRef>)>,
  ty_name: Vec<(Loc, Ty, TyNameSet)>,
  subst: Subst,
}

impl State {
  fn new_ty_var(&mut self, equality: bool) -> TyVar {
    let id = self.next_ty_var;
    self.next_ty_var += 1;
    TyVar { id, equality }
  }

  fn new_sym(&mut self, name: Located<StrRef>) -> Sym {
    let id = Some(name.loc.wrap(self.next_sym));
    self.next_sym += 1;
    Sym { id, name: name.val }
  }

  fn solve(mut self) -> Result<Subst> {
    'outer: for (loc, tv, overloads) in self.overload {
      for name in overloads {
        let mut pre = self.subst.clone();
        if let Ok(()) = unify(&mut pre, loc, Ty::Var(tv), Ty::base(name)) {
          self.subst = pre;
          continue 'outer;
        }
      }
      return Err(loc.wrap(StaticsError::NoSuitableOverload));
    }
    for (loc, mut ty, ty_names) in self.ty_name {
      ty.apply(&self.subst);
      if !ty.ty_names().is_subset(&ty_names) {
        return Err(loc.wrap(StaticsError::TyNameEscape));
      }
    }
    Ok(self.subst)
  }
}

fn instantiate(st: &mut State, ty_scheme: &TyScheme, loc: Loc) -> Ty {
  let mut subst = Subst::default();
  match &ty_scheme.overload {
    None => {
      for &tv in ty_scheme.ty_vars.iter() {
        subst.inner.insert(tv, Ty::Var(st.new_ty_var(tv.equality)));
      }
    }
    Some(overloads) => {
      // NOTE it might be better to have TyScheme be an enum to make these illegal states impossible
      // to represent in the types. But then again, overloading is uncommon since only the standard
      // library is permitted to do it. In fact, the fact that only the standard library may do
      // overloading, and that all overloaded operators are similar in that every one of them has
      // only one overloaded type variable which is meant to be "the same" within a single
      // instantiation, leads to this slightly hacky implementation.
      let mut iter = ty_scheme.ty_vars.iter().copied();
      let tv = iter.next().unwrap();
      assert!(iter.next().is_none());
      assert!(!overloads.is_empty());
      assert!(!tv.equality);
      let new_tv = st.new_ty_var(false);
      subst.inner.insert(tv, Ty::Var(new_tv));
      st.overload.push((loc, new_tv, overloads.clone()));
    }
  }
  let mut ty = ty_scheme.ty.clone();
  ty.apply(&subst);
  ty
}

fn generalize(ty_env: &TyEnv, ty: Ty) -> TyScheme {
  TyScheme {
    ty_vars: ty
      .free_ty_vars()
      .difference(&ty_env.free_ty_vars())
      .copied()
      .collect(),
    ty,
    overload: None,
  }
}

fn bind(subst: &mut Subst, loc: Loc, tv: TyVar, ty: Ty) -> Result<()> {
  if let Ty::Var(other) = ty {
    if tv == other {
      return Ok(());
    }
  }
  if ty.free_ty_vars().contains(&tv) {
    return Err(loc.wrap(StaticsError::Circularity(tv, ty)));
  }
  subst.insert(tv, ty);
  Ok(())
}

fn unify(subst: &mut Subst, loc: Loc, mut lhs: Ty, mut rhs: Ty) -> Result<()> {
  lhs.apply(&subst);
  rhs.apply(&subst);
  match (lhs, rhs) {
    (Ty::Var(tv), rhs) => bind(subst, loc, tv, rhs),
    (lhs, Ty::Var(tv)) => bind(subst, loc, tv, lhs),
    (Ty::Record(rows_l), Ty::Record(rows_r)) => {
      let mut map_l: HashMap<_, _> = rows_l.into_iter().collect();
      let mut map_r: HashMap<_, _> = rows_r.into_iter().collect();
      let keys: HashSet<_> = map_l.keys().chain(map_r.keys()).copied().collect();
      for k in keys {
        match (map_l.remove(&k), map_r.remove(&k)) {
          (Some(ty_l), Some(ty_r)) => unify(subst, loc, ty_l, ty_r)?,
          (Some(..), None) | (None, Some(..)) => {
            return Err(loc.wrap(StaticsError::MissingLabel(k)))
          }
          (None, None) => unreachable!(),
        }
      }
      Ok(())
    }
    (Ty::Arrow(arg_l, res_l), Ty::Arrow(arg_r, res_r)) => {
      unify(subst, loc, *arg_l, *arg_r)?;
      unify(subst, loc, *res_l, *res_r)?;
      Ok(())
    }
    (Ty::Ctor(args_l, name_l), Ty::Ctor(args_r, name_r)) => {
      if name_l != name_r {
        return Err(loc.wrap(StaticsError::HeadMismatch(
          Ty::Ctor(args_l, name_l),
          Ty::Ctor(args_r, name_r),
        )));
      }
      assert_eq!(args_l.len(), args_r.len(), "mismatched Ctor args len");
      for (arg_l, arg_r) in args_l.into_iter().zip(args_r) {
        unify(subst, loc, arg_l, arg_r)?;
      }
      Ok(())
    }
    (lhs @ Ty::Record(..), rhs) | (lhs @ Ty::Arrow(..), rhs) | (lhs @ Ty::Ctor(..), rhs) => {
      Err(loc.wrap(StaticsError::HeadMismatch(lhs, rhs)))
    }
  }
}

fn get_env<'cx>(cx: &'cx Cx, long: &Long<StrRef>) -> Result<&'cx Env> {
  let mut ret = &cx.env;
  for &s in long.structures.iter() {
    ret = match ret.str_env.get(&s.val) {
      None => return Err(s.loc.wrap(StaticsError::Undefined(Item::Structure, s.val))),
      Some(x) => x,
    }
  }
  Ok(ret)
}

fn get_val_info(env: &Env, name: Located<StrRef>) -> Result<&ValInfo> {
  match env.val_env.get(&name.val) {
    None => Err(
      name
        .loc
        .wrap(StaticsError::Undefined(Item::Value, name.val)),
    ),
    Some(val_info) => Ok(val_info),
  }
}

fn tuple_lab(idx: usize) -> Label {
  Label::Num((idx + 1).try_into().unwrap())
}

fn ck_exp(cx: &Cx, st: &mut State, exp: &Located<Exp<StrRef>>) -> Result<Ty> {
  let ret = match &exp.val {
    Exp::DecInt(_) => Ty::INT,
    Exp::HexInt(_) => Ty::INT,
    Exp::DecWord(_) => Ty::WORD,
    Exp::HexWord(_) => Ty::WORD,
    Exp::Real(_) => Ty::REAL,
    Exp::Str(_) => Ty::STRING,
    Exp::Char(_) => Ty::CHAR,
    Exp::LongVid(vid) => {
      let val_info = get_val_info(get_env(cx, vid)?, vid.last)?;
      instantiate(st, &val_info.ty_scheme, exp.loc)
    }
    Exp::Record(rows) => {
      let mut ty_rows = Vec::with_capacity(rows.len());
      let mut keys = HashSet::with_capacity(rows.len());
      for row in rows {
        let ty = ck_exp(cx, st, &row.exp)?;
        if !keys.insert(row.lab.val) {
          return Err(row.lab.loc.wrap(StaticsError::DuplicateLabel(row.lab.val)));
        }
        ty_rows.push((row.lab.val, ty));
      }
      Ty::Record(ty_rows)
    }
    Exp::Select(..) => return Err(exp.loc.wrap(StaticsError::Todo)),
    Exp::Tuple(exps) => {
      let mut ty_rows = Vec::with_capacity(exps.len());
      for (idx, exp) in exps.iter().enumerate() {
        let ty = ck_exp(cx, st, exp)?;
        let lab = tuple_lab(idx);
        ty_rows.push((lab, ty));
      }
      Ty::Record(ty_rows)
    }
    Exp::List(exps) => {
      let mut elem = Ty::Var(st.new_ty_var(false));
      for exp in exps {
        let ty = ck_exp(cx, st, exp)?;
        unify(&mut st.subst, exp.loc, elem.clone(), ty)?;
        elem.apply(&st.subst);
      }
      Ty::list(elem)
    }
    Exp::Sequence(exps) => {
      let mut ret = None;
      for exp in exps {
        ret = Some(ck_exp(cx, st, exp)?);
      }
      ret.unwrap()
    }
    Exp::Let(dec, exps) => {
      let env = ck_dec(cx, st, dec)?;
      let mut cx = cx.clone();
      let ty_names = cx.ty_names.clone();
      cx.o_plus(env);
      let mut last = None;
      for exp in exps {
        last = Some((exp.loc, ck_exp(&cx, st, exp)?));
      }
      let (loc, ty) = last.unwrap();
      st.ty_name.push((loc, ty.clone(), ty_names));
      ty
    }
    Exp::App(func, arg) => {
      let func_ty = ck_exp(cx, st, func)?;
      let arg_ty = ck_exp(cx, st, arg)?;
      let mut ret_ty = Ty::Var(st.new_ty_var(false));
      let arrow_ty = Ty::Arrow(arg_ty.into(), ret_ty.clone().into());
      unify(&mut st.subst, exp.loc, func_ty, arrow_ty)?;
      ret_ty.apply(&st.subst);
      ret_ty
    }
    Exp::InfixApp(lhs, func, rhs) => {
      let val_info = get_val_info(&cx.env, *func)?;
      let func_ty = instantiate(st, &val_info.ty_scheme, exp.loc);
      let lhs_ty = ck_exp(cx, st, lhs)?;
      let rhs_ty = ck_exp(cx, st, rhs)?;
      let mut ret_ty = Ty::Var(st.new_ty_var(false));
      let arrow_ty = Ty::Arrow(
        Ty::Record(vec![(Label::Num(1), lhs_ty), (Label::Num(2), rhs_ty)]).into(),
        ret_ty.clone().into(),
      );
      unify(&mut st.subst, exp.loc, func_ty, arrow_ty)?;
      ret_ty.apply(&st.subst);
      ret_ty
    }
    Exp::Typed(inner, ty) => {
      let mut exp_ty = ck_exp(cx, st, inner)?;
      let ty_ty = ck_ty(cx, st, ty)?;
      unify(&mut st.subst, exp.loc, exp_ty.clone(), ty_ty)?;
      exp_ty.apply(&st.subst);
      exp_ty
    }
    Exp::Andalso(lhs, rhs) | Exp::Orelse(lhs, rhs) => {
      let lhs_ty = ck_exp(cx, st, lhs)?;
      let rhs_ty = ck_exp(cx, st, rhs)?;
      unify(&mut st.subst, lhs.loc, lhs_ty, Ty::BOOL)?;
      unify(&mut st.subst, rhs.loc, rhs_ty, Ty::BOOL)?;
      Ty::BOOL
    }
    Exp::Handle(head, cases) => {
      let mut head_ty = ck_exp(cx, st, head)?;
      let (arg_ty, res_ty) = ck_cases(cx, st, cases)?;
      unify(&mut st.subst, exp.loc, arg_ty, Ty::EXN)?;
      unify(&mut st.subst, exp.loc, head_ty.clone(), res_ty)?;
      head_ty.apply(&st.subst);
      head_ty
    }
    Exp::Raise(exp) => {
      let exp_ty = ck_exp(cx, st, exp)?;
      unify(&mut st.subst, exp.loc, exp_ty, Ty::EXN)?;
      Ty::Var(st.new_ty_var(false))
    }
    Exp::If(cond, then_e, else_e) => {
      let cond_ty = ck_exp(cx, st, cond)?;
      let mut then_ty = ck_exp(cx, st, then_e)?;
      let else_ty = ck_exp(cx, st, else_e)?;
      unify(&mut st.subst, cond.loc, cond_ty, Ty::BOOL)?;
      unify(&mut st.subst, exp.loc, then_ty.clone(), else_ty)?;
      then_ty.apply(&st.subst);
      then_ty
    }
    Exp::While(..) => return Err(exp.loc.wrap(StaticsError::Todo)),
    Exp::Case(head, cases) => {
      let head_ty = ck_exp(cx, st, head)?;
      let (arg_ty, mut res_ty) = ck_cases(cx, st, cases)?;
      unify(&mut st.subst, exp.loc, head_ty, arg_ty)?;
      res_ty.apply(&st.subst);
      res_ty
    }
    Exp::Fn(cases) => {
      let (arg_ty, res_ty) = ck_cases(cx, st, cases)?;
      Ty::Arrow(arg_ty.into(), res_ty.into())
    }
  };
  Ok(ret)
}

fn ck_cases(cx: &Cx, st: &mut State, cases: &Cases<StrRef>) -> Result<(Ty, Ty)> {
  let mut arg_ty = Ty::Var(st.new_ty_var(false));
  let mut res_ty = Ty::Var(st.new_ty_var(false));
  for arm in cases.arms.iter() {
    // TODO clone in loop - expensive?
    let mut cx = cx.clone();
    let (val_env, pat_ty) = ck_pat(&cx, st, &arm.pat)?;
    // TODO what about type variables? The Definition says this should allow new free type variables
    // to enter the Cx, but right now we do nothing with `cx.ty_vars`.
    cx.env.val_env.extend(val_env);
    let exp_ty = ck_exp(&cx, st, &arm.exp)?;
    unify(&mut st.subst, arm.pat.loc, arg_ty.clone(), pat_ty)?;
    unify(&mut st.subst, arm.exp.loc, res_ty.clone(), exp_ty)?;
    arg_ty.apply(&st.subst);
    res_ty.apply(&st.subst);
  }
  Ok((arg_ty, res_ty))
}

fn ck_ty(cx: &Cx, st: &mut State, ty: &Located<AstTy<StrRef>>) -> Result<Ty> {
  let ret = match &ty.val {
    AstTy::TyVar(_) => {
      //
      return Err(ty.loc.wrap(StaticsError::Todo));
    }
    AstTy::Record(rows) => {
      let mut ty_rows = Vec::with_capacity(rows.len());
      let mut keys = HashSet::with_capacity(rows.len());
      for row in rows {
        let ty = ck_ty(cx, st, &row.ty)?;
        if !keys.insert(row.lab.val) {
          return Err(row.lab.loc.wrap(StaticsError::DuplicateLabel(row.lab.val)));
        }
        ty_rows.push((row.lab.val, ty));
      }
      Ty::Record(ty_rows)
    }
    AstTy::Tuple(tys) => {
      let mut ty_rows = Vec::with_capacity(tys.len());
      for (idx, ty) in tys.iter().enumerate() {
        let ty = ck_ty(cx, st, ty)?;
        let lab = tuple_lab(idx);
        ty_rows.push((lab, ty));
      }
      Ty::Record(ty_rows)
    }
    AstTy::TyCon(args, name) => {
      let env = get_env(cx, name)?;
      let ty_info = match env.ty_env.inner.get(&name.last.val) {
        None => {
          return Err(
            name
              .last
              .loc
              .wrap(StaticsError::Undefined(Item::Type, name.last.val)),
          )
        }
        Some(x) => x,
      };
      let want_len = ty_info.ty_fcn.ty_vars.len();
      if want_len != args.len() {
        return Err(
          ty.loc
            .wrap(StaticsError::WrongNumTyArgs(want_len, args.len())),
        );
      }
      let mut subst = Subst::default();
      for (&ty_var, ty) in ty_info.ty_fcn.ty_vars.iter().zip(args.iter()) {
        let ty = ck_ty(cx, st, ty)?;
        subst.inner.insert(ty_var, ty);
      }
      let mut ty = ty_info.ty_fcn.ty.clone();
      ty.apply(&subst);
      ty
    }
    AstTy::Arrow(arg, res) => {
      let arg = ck_ty(cx, st, arg)?;
      let res = ck_ty(cx, st, res)?;
      Ty::Arrow(arg.into(), res.into())
    }
  };
  Ok(ret)
}

fn ck_binding(name: Located<StrRef>) -> Result<()> {
  for &other in [
    StrRef::TRUE,
    StrRef::FALSE,
    StrRef::NIL,
    StrRef::CONS,
    StrRef::REF,
  ]
  .iter()
  {
    if name.val == other {
      return Err(name.loc.wrap(StaticsError::ForbiddenBinding(name.val)));
    }
  }
  Ok(())
}

fn ck_dec(cx: &Cx, st: &mut State, dec: &Located<Dec<StrRef>>) -> Result<Env> {
  let ret = match &dec.val {
    Dec::Val(ty_vars, val_binds) => {
      if let Some(tv) = ty_vars.first() {
        return Err(tv.loc.wrap(StaticsError::Todo));
      }
      let mut val_env = ValEnv::new();
      for val_bind in val_binds {
        if val_bind.rec {
          return Err(dec.loc.wrap(StaticsError::Todo));
        }
        let (other, pat_ty) = ck_pat(cx, st, &val_bind.pat)?;
        for &name in other.keys() {
          ck_binding(val_bind.pat.loc.wrap(name))?;
        }
        env_merge(&mut val_env, other, val_bind.pat.loc)?;
        let exp_ty = ck_exp(cx, st, &val_bind.exp)?;
        unify(&mut st.subst, dec.loc, pat_ty, exp_ty)?;
      }
      val_env.into()
    }
    Dec::Fun(_, _) => {
      //
      return Err(dec.loc.wrap(StaticsError::Todo));
    }
    Dec::Type(ty_binds) => {
      let mut ty_env = TyEnv::default();
      for ty_bind in ty_binds {
        if !ty_bind.ty_vars.is_empty() {
          return Err(dec.loc.wrap(StaticsError::Todo));
        }
        let ty = ck_ty(cx, st, &ty_bind.ty)?;
        let info = TyInfo {
          ty_fcn: TyScheme::mono(ty),
          val_env: ValEnv::new(),
        };
        if ty_env.inner.insert(ty_bind.ty_con.val, info).is_some() {
          return Err(
            ty_bind
              .ty_con
              .loc
              .wrap(StaticsError::Redefined(ty_bind.ty_con.val)),
          );
        }
      }
      ty_env.into()
    }
    Dec::Datatype(dat_binds, ty_binds) => {
      if let Some(x) = ty_binds.first() {
        return Err(x.ty_con.loc.wrap(StaticsError::Todo));
      }
      let mut cx = cx.clone();
      // these two are across all dat_binds.
      let mut ty_env = TyEnv::default();
      let mut val_env = ValEnv::new();
      for dat_bind in dat_binds {
        if let Some(x) = dat_bind.ty_vars.first() {
          return Err(x.loc.wrap(StaticsError::Todo));
        }
        // create a new symbol for the type being generated with this DatBind.
        let sym = st.new_sym(dat_bind.ty_con);
        // tell the original context that this new type does exist, but just with an empty ValEnv.
        // also perform dupe checking on the name of the new type.
        env_ins(
          &mut cx.env.ty_env.inner,
          dat_bind.ty_con,
          TyInfo {
            ty_fcn: TyScheme::mono(Ty::Ctor(Vec::new(), sym)),
            val_env: ValEnv::new(),
          },
        )?;
        cx.ty_names.insert(dat_bind.ty_con.val);
        // this ValEnv is specific to this DatBind.
        let mut bind_val_env = ValEnv::new();
        for con_bind in dat_bind.cons.iter() {
          ck_binding(con_bind.vid)?;
          // the type being defined in this declaration is `ty`.
          let mut ty = Ty::Ctor(Vec::new(), sym);
          if let Some(arg_ty) = &con_bind.ty {
            // if there is an `of t`, then the type of the ctor is `t -> ty`. otherwise, the type of
            // the ctor is just `ty`.
            ty = Ty::Arrow(ck_ty(&cx, st, arg_ty)?.into(), ty.into());
          }
          // insert the ValInfo into the _overall_ ValEnv with dupe checking.
          env_ins(
            &mut val_env,
            con_bind.vid,
            ValInfo::ctor(TyScheme::mono(ty.clone())),
          )?;
          // _also_ insert the ValInfo into the DatBind-specific ValEnv, but this time dupe checking
          // is unnecessary (just assert as a sanity check).
          assert!(bind_val_env
            .insert(con_bind.vid.val, ValInfo::ctor(TyScheme::mono(ty)))
            .is_none());
        }
        // insert the DatBind-specific ValEnv into the _overall_ TyEnv. we already did dupe checking
        // when inserting the "fake" TyInfo into the global Cx earlier, so this time just assert as
        // a sanity check.
        assert!(ty_env
          .inner
          .insert(
            dat_bind.ty_con.val,
            TyInfo {
              ty_fcn: TyScheme::mono(Ty::Ctor(Vec::new(), sym)),
              val_env: bind_val_env,
            },
          )
          .is_none());
      }
      Env {
        ty_env,
        val_env,
        str_env: StrEnv::new(),
      }
    }
    Dec::DatatypeCopy(_, _) => {
      //
      return Err(dec.loc.wrap(StaticsError::Todo));
    }
    Dec::Abstype(..) => return Err(dec.loc.wrap(StaticsError::Todo)),
    Dec::Exception(_) => {
      //
      return Err(dec.loc.wrap(StaticsError::Todo));
    }
    Dec::Local(_, _) => {
      //
      return Err(dec.loc.wrap(StaticsError::Todo));
    }
    Dec::Open(_) => {
      //
      return Err(dec.loc.wrap(StaticsError::Todo));
    }
    Dec::Seq(decs) => {
      // TODO clone in loop - expensive?
      let mut cx = cx.clone();
      let mut ret = Env::default();
      for dec in decs {
        cx.o_plus(ret.clone());
        ret.extend(ck_dec(&cx, st, dec)?);
      }
      ret
    }
    Dec::Infix(..) | Dec::Infixr(..) | Dec::Nonfix(..) => Env::default(),
  };
  Ok(ret)
}

fn env_ins<T>(map: &mut HashMap<StrRef, T>, key: Located<StrRef>, val: T) -> Result<()> {
  if map.insert(key.val, val).is_some() {
    Err(key.loc.wrap(StaticsError::Redefined(key.val)))
  } else {
    Ok(())
  }
}

fn env_merge<T>(lhs: &mut HashMap<StrRef, T>, rhs: HashMap<StrRef, T>, loc: Loc) -> Result<()> {
  for (key, val) in rhs {
    env_ins(lhs, loc.wrap(key), val)?;
  }
  Ok(())
}

fn ck_pat(cx: &Cx, st: &mut State, pat: &Located<Pat<StrRef>>) -> Result<(ValEnv, Ty)> {
  let ret = match &pat.val {
    Pat::Wildcard => (ValEnv::new(), Ty::Var(st.new_ty_var(false))),
    Pat::DecInt(_) => (ValEnv::new(), Ty::INT),
    Pat::HexInt(_) => (ValEnv::new(), Ty::INT),
    Pat::DecWord(_) => (ValEnv::new(), Ty::WORD),
    Pat::HexWord(_) => (ValEnv::new(), Ty::WORD),
    Pat::Str(_) => (ValEnv::new(), Ty::STRING),
    Pat::Char(_) => (ValEnv::new(), Ty::CHAR),
    Pat::LongVid(vid) => {
      let ty_scheme = if vid.structures.is_empty() {
        None
      } else {
        cx.env.val_env.get(&vid.last.val).and_then(|val_info| {
          if val_info.id_status == IdStatus::Val {
            Some(&val_info.ty_scheme)
          } else {
            None
          }
        })
      };
      match ty_scheme {
        None => {
          // TODO should this be TyScheme::mono?
          let a = Ty::Var(st.new_ty_var(false));
          let val_info = ValInfo::val(TyScheme::mono(a.clone()));
          (hashmap![vid.last.val => val_info], a)
        }
        Some(ty_scheme) => {
          // TODO do we need to check ty_scheme yields a ConsType? e.g. `fn op:: => op::` may be
          // problematic
          (ValEnv::new(), instantiate(st, ty_scheme, pat.loc))
        }
      }
    }
    Pat::Record(rows, rest_loc) => {
      if let Some(loc) = rest_loc {
        return Err(loc.wrap(StaticsError::Todo));
      }
      let mut ve = ValEnv::new();
      let mut ty_rows = Vec::with_capacity(rows.len());
      let mut keys = HashSet::with_capacity(rows.len());
      for row in rows {
        let (other_ve, ty) = ck_pat(cx, st, &row.pat)?;
        if !keys.insert(row.lab.val) {
          return Err(row.lab.loc.wrap(StaticsError::DuplicateLabel(row.lab.val)));
        }
        env_merge(&mut ve, other_ve, row.pat.loc)?;
        ty_rows.push((row.lab.val, ty));
      }
      (ve, Ty::Record(ty_rows))
    }
    Pat::Tuple(pats) => {
      let mut ve = ValEnv::new();
      let mut ty_rows = Vec::with_capacity(pats.len());
      for (idx, pat) in pats.iter().enumerate() {
        let (other_ve, ty) = ck_pat(cx, st, pat)?;
        let lab = tuple_lab(idx);
        env_merge(&mut ve, other_ve, pat.loc)?;
        ty_rows.push((lab, ty));
      }
      (ve, Ty::Record(ty_rows))
    }
    Pat::List(pats) => {
      let mut elem = Ty::Var(st.new_ty_var(false));
      let mut ve = ValEnv::new();
      for pat in pats {
        let (other_ve, ty) = ck_pat(cx, st, pat)?;
        env_merge(&mut ve, other_ve, pat.loc)?;
        unify(&mut st.subst, pat.loc, elem.clone(), ty)?;
        elem.apply(&st.subst);
      }
      (ve, Ty::list(elem))
    }
    Pat::Ctor(vid, arg) => {
      let val_info = get_val_info(get_env(cx, vid)?, vid.last)?;
      if val_info.id_status == IdStatus::Val {
        return Err(vid.loc().wrap(StaticsError::ValAsPat));
      }
      let (val_env, arg_ty) = ck_pat(cx, st, arg)?;
      let ctor_ty = instantiate(st, &val_info.ty_scheme, pat.loc);
      let mut ret_ty = Ty::Var(st.new_ty_var(false));
      unify(
        &mut st.subst,
        pat.loc,
        ctor_ty,
        Ty::Arrow(arg_ty.into(), ret_ty.clone().into()),
      )?;
      ret_ty.apply(&st.subst);
      (val_env, ret_ty)
    }
    Pat::InfixCtor(lhs, vid, rhs) => {
      let val_info = get_val_info(&cx.env, *vid)?;
      if val_info.id_status == IdStatus::Val {
        return Err(vid.loc.wrap(StaticsError::ValAsPat));
      }
      let func_ty = instantiate(st, &val_info.ty_scheme, pat.loc);
      let (mut val_env, lhs_ty) = ck_pat(cx, st, lhs)?;
      let (other_ve, rhs_ty) = ck_pat(cx, st, rhs)?;
      env_merge(&mut val_env, other_ve, pat.loc)?;
      let mut ret_ty = Ty::Var(st.new_ty_var(false));
      let arrow_ty = Ty::Arrow(
        Ty::Record(vec![(Label::Num(1), lhs_ty), (Label::Num(2), rhs_ty)]).into(),
        ret_ty.clone().into(),
      );
      unify(&mut st.subst, pat.loc, func_ty, arrow_ty)?;
      ret_ty.apply(&st.subst);
      (val_env, ret_ty)
    }
    Pat::Typed(inner_pat, ty) => {
      let (val_env, mut pat_ty) = ck_pat(cx, st, inner_pat)?;
      let ty = ck_ty(cx, st, ty)?;
      unify(&mut st.subst, pat.loc, pat_ty.clone(), ty)?;
      pat_ty.apply(&st.subst);
      (val_env, pat_ty)
    }
    Pat::As(vid, ty, inner_pat) => {
      if cx
        .env
        .val_env
        .get(&vid.val)
        .map_or(false, |x| x.id_status != IdStatus::Val)
      {
        return Err(vid.loc.wrap(StaticsError::NonVarInAs(vid.val)));
      }
      let (mut val_env, mut pat_ty) = ck_pat(cx, st, inner_pat)?;
      if let Some(ty) = ty {
        let ty = ck_ty(cx, st, ty)?;
        unify(&mut st.subst, pat.loc, pat_ty.clone(), ty)?;
        pat_ty.apply(&st.subst);
      }
      let val_info = ValInfo::val(TyScheme::mono(pat_ty.clone()));
      env_ins(&mut val_env, *vid, val_info)?;
      (val_env, pat_ty)
    }
  };
  Ok(ret)
}

fn ck_top_dec(bs: Basis, st: &mut State, top_dec: &Located<TopDec<StrRef>>) -> Result<Basis> {
  match &top_dec.val {
    TopDec::StrDec(str_dec) => match &str_dec.val {
      StrDec::Dec(dec) => {
        let mut cx = Cx {
          ty_names: bs.ty_names,
          ty_vars: TyVarSet::new(),
          env: bs.env,
        };
        cx.o_plus(ck_dec(&cx, st, dec)?);
        Ok(Basis {
          env: cx.env,
          ty_names: cx.ty_names,
          ..bs
        })
      }
      StrDec::Structure(_) => Err(top_dec.loc.wrap(StaticsError::Todo)),
      StrDec::Local(_, _) => Err(top_dec.loc.wrap(StaticsError::Todo)),
      StrDec::Seq(_) => Err(top_dec.loc.wrap(StaticsError::Todo)),
    },
    TopDec::SigDec(_) => Err(top_dec.loc.wrap(StaticsError::Todo)),
    TopDec::FunDec(_) => Err(top_dec.loc.wrap(StaticsError::Todo)),
  }
}

fn prim_ty_info(ty: Ty) -> TyInfo {
  TyInfo {
    ty_fcn: TyScheme::mono(ty),
    val_env: ValEnv::new(),
  }
}

fn bool_val_env() -> ValEnv {
  hashmap![
    StrRef::TRUE => ValInfo::ctor(TyScheme::mono(Ty::BOOL)),
    StrRef::FALSE => ValInfo::ctor(TyScheme::mono(Ty::BOOL)),
  ]
}

fn list_val_env(st: &mut State) -> ValEnv {
  let a = st.new_ty_var(false);
  let nil = ValInfo::ctor(TyScheme {
    ty_vars: vec![a],
    ty: Ty::list(Ty::Var(a)),
    overload: None,
  });
  let a = st.new_ty_var(false);
  let cons = ValInfo::ctor(TyScheme {
    ty_vars: vec![a],
    ty: Ty::Arrow(
      Ty::Record(vec![
        (Label::Num(1), Ty::Var(a)),
        (Label::Num(2), Ty::list(Ty::Var(a))),
      ])
      .into(),
      Ty::list(Ty::Var(a)).into(),
    ),
    overload: None,
  });
  hashmap![StrRef::NIL => nil, StrRef::CONS => cons]
}

fn ref_val_env(st: &mut State) -> ValEnv {
  let a = st.new_ty_var(false);
  let ref_ = ValInfo::ctor(TyScheme {
    ty_vars: vec![a],
    ty: Ty::Arrow(Ty::Var(a).into(), Ty::ref_(Ty::Var(a)).into()),
    overload: None,
  });
  hashmap![StrRef::REF => ref_]
}

fn order_val_env() -> ValEnv {
  hashmap![
    StrRef::LESS => ValInfo::ctor(TyScheme::mono(Ty::ORDER)),
    StrRef::EQUAL => ValInfo::ctor(TyScheme::mono(Ty::ORDER)),
    StrRef::GREATER => ValInfo::ctor(TyScheme::mono(Ty::ORDER)),
  ]
}

fn overloaded(st: &mut State, overloads: Vec<StrRef>) -> ValInfo {
  let a = st.new_ty_var(false);
  ValInfo::val(TyScheme {
    ty_vars: vec![a],
    ty: Ty::Arrow(
      Ty::Record(vec![
        (Label::Num(1), Ty::Var(a)),
        (Label::Num(2), Ty::Var(a)),
      ])
      .into(),
      Ty::Var(a).into(),
    ),
    overload: Some(overloads),
  })
}

fn overloaded_cmp(st: &mut State) -> ValInfo {
  let a = st.new_ty_var(false);
  ValInfo::val(TyScheme {
    ty_vars: vec![a],
    ty: Ty::Arrow(
      Ty::Record(vec![
        (Label::Num(1), Ty::Var(a)),
        (Label::Num(2), Ty::Var(a)),
      ])
      .into(),
      Ty::base(StrRef::BOOL).into(),
    ),
    overload: Some(vec![
      StrRef::INT,
      StrRef::WORD,
      StrRef::REAL,
      StrRef::STRING,
      StrRef::CHAR,
    ]),
  })
}

fn std_lib() -> (Basis, State) {
  let real_int = || vec![StrRef::INT, StrRef::REAL];
  let word_int = || vec![StrRef::INT, StrRef::WORD];
  let num = || vec![StrRef::INT, StrRef::WORD, StrRef::REAL];
  let mut st = State::default();
  let bs = Basis {
    ty_names: hashset![
      StrRef::BOOL,
      StrRef::INT,
      StrRef::REAL,
      StrRef::STRING,
      StrRef::CHAR,
      StrRef::WORD,
      StrRef::LIST,
      StrRef::REF,
      StrRef::EXN,
      StrRef::ORDER,
    ],
    fun_env: FunEnv::new(),
    sig_env: SigEnv::new(),
    env: Env {
      str_env: StrEnv::new(),
      ty_env: TyEnv {
        inner: hashmap![
          StrRef::UNIT => prim_ty_info(Ty::Record(Vec::new())),
          StrRef::BOOL => TyInfo {
            ty_fcn: TyScheme::mono(Ty::BOOL),
            val_env: bool_val_env(),
          },
          StrRef::INT => prim_ty_info(Ty::INT),
          StrRef::REAL => prim_ty_info(Ty::REAL),
          StrRef::STRING => prim_ty_info(Ty::STRING),
          StrRef::CHAR => prim_ty_info(Ty::CHAR),
          StrRef::WORD => prim_ty_info(Ty::WORD),
          StrRef::LIST => TyInfo {
            ty_fcn: {
              let a = st.new_ty_var(false);
              TyScheme {
                ty_vars: vec![a],
                ty: Ty::list(Ty::Var(a)),
                overload: None,
              }
            },
            val_env: list_val_env(&mut st),
          },
          StrRef::REF => TyInfo {
            ty_fcn: {
              let a = st.new_ty_var(false);
              TyScheme {
                ty_vars: vec![a],
                ty: Ty::ref_(Ty::Var(a)),
                overload: None,
              }
            },
            val_env: ref_val_env(&mut st),
          },
          StrRef::EXN => prim_ty_info(Ty::EXN),
          StrRef::ORDER => TyInfo {
            ty_fcn: TyScheme::mono(Ty::ORDER),
            val_env: order_val_env(),
          },
        ],
      },
      val_env: bool_val_env()
        .into_iter()
        .chain(list_val_env(&mut st))
        .chain(ref_val_env(&mut st))
        .chain(order_val_env())
        .chain(hashmap![
          StrRef::EQ => ValInfo::val({
            let a = st.new_ty_var(true);
            TyScheme {
              ty_vars: vec![a],
              ty: Ty::Arrow(
                Ty::Record(vec![
                  (Label::Num(1), Ty::Var(a)),
                  (Label::Num(2), Ty::Var(a)),
                ])
                .into(),
                Ty::BOOL.into(),
              ),
              overload: None,
            }
          }),
          StrRef::ASSIGN => ValInfo::val({
            let a = st.new_ty_var(false);
            TyScheme {
              ty_vars: vec![a],
              ty: Ty::Arrow(
                Ty::Record(vec![
                  (
                    Label::Num(1),
                    Ty::Ctor(vec![Ty::Var(a)], Sym::base(StrRef::REF)),
                  ),
                  (Label::Num(2), Ty::Var(a)),
                ])
                .into(),
                Ty::Record(Vec::new()).into(),
              ),
              overload: None,
            }
          }),
          StrRef::MATCH => ValInfo::exn(),
          StrRef::BIND => ValInfo::exn(),
          StrRef::ABS => overloaded(&mut st, real_int()),
          StrRef::TILDE => overloaded(&mut st, real_int()),
          StrRef::DIV => overloaded(&mut st, word_int()),
          StrRef::MOD => overloaded(&mut st, word_int()),
          StrRef::STAR => overloaded(&mut st, word_int()),
          StrRef::SLASH => overloaded(&mut st, vec![StrRef::REAL]),
          StrRef::PLUS => overloaded(&mut st, num()),
          StrRef::MINUS => overloaded(&mut st, num()),
          // the Definition states these have type numtxt * numtxt -> numtxt but they really should
          // be numtxt * numtxt -> bool.
          StrRef::LT => overloaded_cmp(&mut st),
          StrRef::GT => overloaded_cmp(&mut st),
          StrRef::LT_EQ => overloaded_cmp(&mut st),
          StrRef::GT_EQ => overloaded_cmp(&mut st),
        ])
        .collect(),
    },
  };
  (bs, st)
}
