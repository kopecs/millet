//! Statics.
//!
//! With help from [this article][1].
//!
//! [1]: http://dev.stephendiehl.com/fun/006_hindley_milner.html

mod ck;
mod std_lib;
mod types;

use crate::ast::TopDec;
use crate::intern::StrRef;
use crate::loc::Located;
use crate::statics::types::Result;

/// Performs static analysis. Returns `Ok(())` iff everything typechecks.
pub fn get(top_decs: &[Located<TopDec<StrRef>>]) -> Result<()> {
  let (mut bs, mut st) = std_lib::get();
  for top_dec in top_decs {
    ck::ck_top_dec(&mut bs, &mut st, top_dec)?;
  }
  bs.apply(&st.subst, &mut st.sym_tys);
  assert!(bs.free_ty_vars(&st.sym_tys).is_empty());
  Ok(())
}
