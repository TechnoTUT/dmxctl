//! 遠隔の操作で押している間だけ続ける割り当て。離したときに元へ戻すための記録。
use crate::engine::Core;
use crate::project::Action;
use serde_json::json;
use std::time::Instant;

pub struct Hold {
    action: Action,
    until: Instant,
    feedback: Feedback,
}

#[derive(Clone)]
pub enum Feedback { None, Note(u8), Key(u8) }

impl Core {
    /// 割り当ての `id` を、`duration` のあいだ押している扱いにする。`feedback` は離したときに消す表示
    pub fn hold_remote_control(&mut self, id: u64, action: Action, duration: std::time::Duration, feedback: Feedback) {
        let until = self.now() + duration;
        self.rt.remote_holds.insert(id, Hold { action, until, feedback });
    }

    /// 割り当ての `id` を、離す処理をせずに押している扱いから外す
    pub fn forget_remote_control(&mut self, id: u64) {
        self.rt.remote_holds.remove(&id);
    }

    /// 割り当ての `id` を押している扱いか
    pub fn is_remote_held(&self, id: u64) -> bool {
        self.rt.remote_holds.contains_key(&id)
    }

    /// 割り当ての `id` を、離した扱いにする。`source` は記録に残す理由
    pub fn release_remote_control(&mut self, id: u64, source: &str) {
        if let Some(hold) = self.rt.remote_holds.remove(&id) {
            self.release_hold(id, hold, source);
        }
    }

    /// 期限が切れた押している間の記録を、離した扱いにする
    pub fn expire_remote_controls(&mut self,now:Instant) {
        let expired:Vec<_>=self.rt.remote_holds.iter().filter(|(_,hold)|now>=hold.until).map(|(&id,_)|id).collect();
        for id in expired {if let Some(hold)=self.rt.remote_holds.remove(&id) {self.release_hold(id,hold,"lease_expired");}}
    }

    /// 押している間の記録を、すべて離した扱いにする。プロジェクトを編集するときに呼ぶ
    pub fn release_remote_controls(&mut self) {
        let holds=std::mem::take(&mut self.rt.remote_holds);
        for (id,hold) in holds {self.release_hold(id,hold,"project_edit");}
    }

    fn release_hold(&mut self,id:u64,hold:Hold,source:&str) {
        let index=self.project.bindings.iter().position(|b|b.id==id).unwrap_or(usize::MAX);
        self.rt.pad_state.insert(index,true);
        self.apply_action(index,hold.action,0.0,false);
        if index==usize::MAX {self.rt.pad_state.remove(&index);}
        match hold.feedback {Feedback::None=>{},Feedback::Note(n)=>self.rt.notes_down[n as usize]=false,Feedback::Key(n)=>self.rt.keys_down.retain(|&k|k!=n)}
        if let Some(recorder)=&self.rt.recorder {
            recorder.emit(json!({"type":"command","source":source,"command":{"cmd":"button","control":format!("binding:{id}"),"event":"release"},
                "applies_from_frame":recorder.next_frame(),"applies_from_engine_frame":self.rt.frame_id+1,
                "t_ms":self.now().saturating_duration_since(self.rt.t0).as_millis() as u64}));
        }
    }
}
