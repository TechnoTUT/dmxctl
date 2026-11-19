//! 遠隔の操作で押している間だけ続ける割り当て。離したときに元へ戻すための記録。
use crate::core::project::Action;
use std::time::Instant;

pub struct Hold {
    pub(crate) action: Action,
    pub(crate) until: Instant,
    pub(crate) feedback: Feedback,
}

#[derive(Clone)]
pub(crate) enum Feedback { None, Note(u8), Key(u8) }
