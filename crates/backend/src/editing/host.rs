//! Host の MIDI 機器の合わせ込みと、保存してある設定との差分。
use super::*;

impl crate::host::Host {
    /// プロジェクトのMIDI機器(midi_input)に、つないでいる機器を合わせる
    pub(crate) fn sync_midi_choice(&mut self)->anyhow::Result<()> {
        let name=dmxctl_core::engine::lock(&self.core).project.midi_input.clone();
        self.revisions.midi_synced+=1;
        if !self.launch.inputs {return Ok(());}
        if self.midi.connected.as_deref()==Some(name.as_str()) {return Ok(());}
        self.midi.disconnect();
        if !name.is_empty() {self.midi.connect(&self.core,self.ctx.clone(),&name).map_err(anyhow::Error::msg)?;}
        Ok(())
    }

    pub(crate) fn project_diff(&self)->anyhow::Result<Value> {
        let text=std::fs::read_to_string(&self.project_path)?;
        let mut saved:Project=serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        saved.ensure_binding_ids();
        let current=dmxctl_core::engine::lock(&self.core).project.clone();
        let pin_changed=saved.web_pin!=current.web_pin;
        let saved=redact(&serde_json::to_value(saved)?);
        let current=redact(&serde_json::to_value(&current)?);
        let mut changes=Vec::new();
        differences("",&saved,&current,&mut changes);
        Ok(json!({"changes":changes,"redacted_changes":{"web_pin":pin_changed},"project":self.project_path}))
    }

    pub(crate) fn reload_checked(&mut self,path:Option<&std::path::Path>,remote:bool,guard_safety:bool,dry_run:bool)->anyhow::Result<Value> {
        let path=path.unwrap_or(&self.project_path).to_path_buf();
        let text=std::fs::read_to_string(&path)?;
        self.reload_text(&text,&path.display().to_string(),remote,guard_safety,dry_run)
    }

    /// プロジェクトの中身(JSON)を読み込む。保存先(project_path)は変えない。読み込む前の状態はバックアップに残す。
    /// `source` は結果に出す読み込み元(ファイルの場所や名前)。
    /// `remote`: 自動操作の窓口(ctl・MCP)から。安全の確認と厳しい検証をする。
    /// `guard_safety`: LAN のクライアントから(`--remote-allow-safety-edit` なら false)。安全の設定が変わる読み込みを拒む
    pub(crate) fn reload_text(&mut self,text:&str,source:&str,remote:bool,guard_safety:bool,dry_run:bool)->anyhow::Result<Value> {
        let path=source;
        // 自動操作の窓口(ctl・MCP)は厳しい検証も。GUIの「読み込み」は今までどおり厳しい検証をしない(GUIで保存したファイルを読み戻せるように)
        let check=|core:&Core,project:&Project|->anyhow::Result<()> {
            if remote {safety_unchanged(&core.project,project,&core.lib)?;validate(project,&core.lib)?;}
            else if guard_safety {safety_unchanged(&core.project,project,&core.lib).map_err(|e|anyhow::anyhow!("{}({e})",crate::project_patch::SAFETY_READ_ONLY))?;}
            Ok(())
        };
        let mut project:Project=serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        let previous={
            let core=dmxctl_core::engine::lock(&self.core);
            ensure_idle(&core)?;
            project.next_binding_id=project.next_binding_id.max(core.project.next_binding_id);
            project.next_id=project.next_id.max(core.project.next_id);
            project.ensure_binding_ids();
            // GUIの「読み込み」は今までどおり厳しい検証をしない(GUIで保存したファイルを読み戻せるように)
            check(&core,&project)?;
            core.project.clone()
        };
        if dry_run {return Ok(json!({"applied":false,"valid":true,"source":path}));}
        let dir=self.project_path.parent().unwrap_or(std::path::Path::new(".")).join("backups");
        std::fs::create_dir_all(&dir)?;
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let backup=dir.join(format!("before-load-{stamp}-{}.json",std::process::id()));
        let file=std::fs::OpenOptions::new().write(true).create_new(true).open(&backup)?;
        let mut writer=std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer,&previous)?;
        std::io::Write::flush(&mut writer)?;
        let old_midi=previous.midi_input;
        let new_midi=project.midi_input.clone();
        {
            let mut core=dmxctl_core::engine::lock(&self.core);
            ensure_idle(&core)?;
            // バックアップを書いている間に、別のクライアントが設定を変えていることがある。反映する直前の設定とも比べる
            check(&core,&project)?;
            commit_with_arm(&mut core,project,false);
        }
        let midi_error=if old_midi!=new_midi {self.sync_midi_choice().err().map(|e|e.to_string())}else{None};
        self.apply_services();
        self.revisions.reloads+=1;
        Ok(json!({"applied":true,"source":path,"backup":backup,"project":self.project_path,"midi_error":midi_error}))
    }
}

pub(super) fn differences(path:&str,before:&Value,after:&Value,out:&mut Vec<Value>) {
    if before==after {return;}
    if let (Some(a),Some(b))=(before.as_object(),after.as_object()) {
        let keys:std::collections::BTreeSet<_>=a.keys().chain(b.keys()).collect();
        for key in keys {differences(&format!("{path}/{}",key.replace('~',"~0").replace('/',"~1")),a.get(key).unwrap_or(&Value::Null),b.get(key).unwrap_or(&Value::Null),out);}
    }else if let (Some(a),Some(b))=(before.as_array(),after.as_array()) {
        if a.len()==b.len() {for (i,(a,b)) in a.iter().zip(b).enumerate(){differences(&format!("{path}/{i}"),a,b,out);}}
        else {out.push(json!({"path":path,"before":before,"after":after}));}
    }else {out.push(json!({"path":path,"before":before,"after":after}));}
}
