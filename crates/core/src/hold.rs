//! 遠隔の操作で押している間だけ続ける割り当て。離したときに元へ戻すための記録。
use crate::engine::Core;
use crate::project::Action;
use serde_json::json;
use std::time::Instant;

pub struct Hold {
    pub action: Action,
    pub until: Instant,
    pub feedback: Feedback,
}

#[derive(Clone)]
pub enum Feedback { None, Note(u8), Key(u8) }

impl Core {
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

    pub fn release_hold(&mut self,id:u64,hold:Hold,source:&str) {
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
