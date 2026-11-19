//! 機材定義の保存・削除。安全の設定を巻き込む変更は断る。
use super::*;

pub(super) fn guard_signature(profile:&dmxctl_core::fixture::Profile)->Value {
    json!({"needs_arm":profile.needs_arm,"modes":profile.modes.iter().map(|mode|json!({
        "name":mode.name,"channels":mode.channels.iter().map(|ch|json!({
            "attr":ch.attr,"fine_of":ch.fine_of,"safe":ch.safe,"forbid":ch.forbid
        })).collect::<Vec<_>>()
    })).collect::<Vec<_>>()})
}

pub(super) fn guarded(profile:&dmxctl_core::fixture::Profile)->bool {
    profile.needs_arm || profile.modes.iter().flat_map(|m|&m.channels).any(|ch|ch.safe.is_some() || !ch.forbid.is_empty())
}

pub(super) fn profile_schema_compatible(project:&Project,old:&Library,new:&Library,id:&str)->anyhow::Result<()> {
    for fixture in project.fixtures.iter().filter(|f|f.profile==id) {
        let old=old.mode(id,fixture.mode).context("unknown existing fixture mode")?;
        let new=new.mode(id,fixture.mode).context("unpatch and save before removing an in-use profile/mode")?;
        let schema=|mode:&dmxctl_core::fixture::Mode|json!(mode.channels.iter().map(|c|(&c.attr,c.fine_of)).collect::<Vec<_>>());
        ensure!(schema(old)==schema(new),"unpatch the fixture and save the project before changing its profile channel structure");
    }
    Ok(())
}

pub(super) fn saved_profile_compatible(core:&Core,candidate:&Library,id:&str)->anyhow::Result<()> {
    profile_schema_compatible(&core.project,&core.lib,candidate,id)?;
    if let Some(path)=&core.rt.project_path {
        let text=std::fs::read_to_string(path)?;
        let saved:Project=serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        profile_schema_compatible(&saved,&core.lib,candidate,id)?;
        crate::commands::validate(&saved,candidate)?;
    }
    Ok(())
}

pub trait CoreEditing {
    fn save_profile(&mut self,profile:&dmxctl_core::fixture::Profile,remote:bool,dry_run:bool)->anyhow::Result<Value>;
    fn delete_profile(&mut self,id:&str,remote:bool,dry_run:bool)->anyhow::Result<Value>;
}

impl CoreEditing for dmxctl_core::engine::Core {
    fn save_profile(&mut self,profile:&dmxctl_core::fixture::Profile,remote:bool,dry_run:bool)->anyhow::Result<Value> {
        ensure_idle(self)?;
        ensure!(!profile.id.trim().is_empty(),"profile ID is empty");
        let report=profile.validate();ensure!(report.errors.is_empty(),"invalid profile: {}",report.errors.join("; "));
        if let Some(old)=self.lib.get(&profile.id) {
            if remote && (guarded(old) || guarded(profile)) {ensure!(guard_signature(old)==guard_signature(profile),"profile safety definitions are read-only from CLI");}
        }
        let candidate=std::sync::Arc::new(self.lib.preview_user(profile));
        saved_profile_compatible(self,&candidate,&profile.id)?;
        let mut trial=Core::new(self.project.clone(),self.lib.clone());trial.replace_library(candidate.clone());
        validate(&trial.project,&candidate)?;
        if dry_run {return Ok(json!({"saved":false,"valid":true,"warnings":report.warnings}));}
        let path=self.lib.user_dir.join(format!("{}.json",dmxctl_core::fixture::slug(&profile.id)));
        if path.exists() && !self.lib.has_user_file(&profile.id) {
            let existing:dmxctl_core::fixture::Profile=serde_json::from_str(&std::fs::read_to_string(&path)?)?;
            ensure!(existing.id==profile.id,"profile filename collides with another ID");
        }
        let path=self.lib.save_user(profile).map_err(anyhow::Error::msg)?;
        let mut library=(*candidate).clone();
        library.register_user_file(&profile.id,path.clone());
        self.replace_library(std::sync::Arc::new(library));
        Ok(json!({"saved":true,"path":path,"warnings":report.warnings}))
    }

    fn delete_profile(&mut self,id:&str,remote:bool,dry_run:bool)->anyhow::Result<Value> {
        ensure_idle(self)?;
        let old=self.lib.get(id).context("unknown profile")?;
        ensure!(self.lib.has_user_file(id),"profile has no user override to delete");
        let candidate=std::sync::Arc::new(self.lib.preview_delete(id));
        saved_profile_compatible(self,&candidate,id)?;
        if remote && guarded(old) {
            let fallback=candidate.get(id).context("protected profiles cannot be deleted from CLI")?;
            ensure!(guard_signature(old)==guard_signature(fallback),"deleting this override would change safety definitions");
        }
        let mut trial=Core::new(self.project.clone(),self.lib.clone());trial.replace_library(candidate.clone());
        validate(&trial.project,&candidate)?;
        if !dry_run {self.lib.delete_user(id).map_err(anyhow::Error::msg)?;self.replace_library(candidate);}
        Ok(json!({"deleted":!dry_run,"id":id,"builtin_fallback":self.lib.is_builtin(id)}))
    }
}
