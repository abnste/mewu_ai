// SPDX-License-Identifier: MPL-2.0
//! Generic migration access to the historical memory ledger. No provider HTTP
//! protocol, package ID, bank naming convention or rendering belongs here.
use super::*;

fn settings(context:&Context)->Result<(),String> {
    if !context.surface_owner.as_ref().is_some_and(|(_,owner)|owner.scene.is_empty()) {
        return Err("请在插件设置界面迁移记忆档案".into());
    }
    Ok(())
}
fn binding(app:&AppHandle,context:&Context,id:&str,settings_only:bool)->Result<mewu_core::MemoryBindingView,String> {
    if settings_only {settings(context)?;}
    let host=app.state::<Host>();let engine=host.lock()?;
    let binding=engine.store.memory_binding(id).map_err(|_|"记忆档案不存在")?;
    engine.store.memory_archive_route(&binding.agent_id,id).map_err(|_|"记忆档案不可用")?;
    if context.surface_owner.is_none() {
        if context.scope!=binding.agent_id || !context.memory_enabled_required {
            return Err("记忆档案不属于当前 Agent 授权".into());
        }
    } else { settings(context)?; }
    Ok(binding)
}
fn expected(binding:&mewu_core::MemoryBindingView,params:&Value)->Result<(),String> {
    if params.get("expectedRevision").and_then(Value::as_u64)!=Some(binding.revision) {
        return Err("记忆档案已变化，请重新载入".into());
    }
    Ok(())
}
pub(super) fn call(app:&AppHandle,context:&Context,method:&str,params:Value)->Result<Value,String> {
    if method=="memory.archive.list" {
        settings(context)?;
        let after=params.get("cursor").and_then(Value::as_str).unwrap_or("");
        let bindings=app.state::<ExtensionHost>().store()?.bindings()?;
        let host=app.state::<Host>();let engine=host.lock()?;
        let mut rows=engine.store.memory_archive_bindings(after,26).map_err(|_|"无法读取记忆档案")?;
        let more=rows.len()>25;rows.truncate(25);
        let cursor=if more {rows.last().map(|b|b.id.clone())}else{None};
        let snapshot=engine.store.snapshot();
        let mut entries=Vec::new();
        for b in rows {
            let Some(agent)=snapshot.agents.iter().find(|a|a.id==b.agent_id&&!a.archived)else{continue};
            let route=engine.store.memory_archive_route(&b.agent_id,&b.id).map_err(|_|"记忆档案不可用")?;
            entries.push(json!({"binding":b,"bankId":route.bank_id,"agentName":agent.name,"selected":agent.memory_provider.binding_id.as_deref()==Some(b.id.as_str()),"bindingRevision":bindings.iter().find(|v|v.agent_id==b.agent_id&&v.kind=="memory").map_or(0,|v|v.revision)}));
        }
        return Ok(json!({"entries":entries,"nextCursor":cursor}));
    }
    let id=string(&params,"bindingId")?;
    let b=binding(app,context,id,matches!(method,"memory.archive.credential"|"memory.archive.activate"|"memory.archive.page"))?;
    match method {
        "memory.archive.credential"=>{
            expected(&b,&params)?;
            let route={let host=app.state::<Host>();let engine=host.lock()?;
                engine.store.memory_archive_route(&b.agent_id,id).map_err(|_|"记忆档案不可用")?};
            secrets::import_archive(app,context,&route,params.get("secretRevision").and_then(Value::as_u64).ok_or("缺少凭据版本")?)
        },
        "memory.archive.page"=>{
            let host=app.state::<Host>();let engine=host.lock()?;
            serde_json::to_value(engine.store.external_memory_page(&b.agent_id,id,"",params.get("cursor").and_then(Value::as_str),25).map_err(|_|"无法读取记忆档案")?).map_err(|_|"记忆档案无效".into())
        },
        "memory.archive.lookup"=>{
            let documents:Vec<String>=serde_json::from_value(params.get("documentIds").cloned().ok_or("缺少文档标识")?).map_err(|_|"文档标识无效")?;
            let host=app.state::<Host>();let engine=host.lock()?;
            serde_json::to_value(engine.store.memory_archive_documents(&b.agent_id,id,&documents).map_err(|_|"无法读取记忆档案")?).map_err(|_|"记忆档案无效".into())
        },
        "memory.archive.forget"=>{
            context.permission("memory.archive.write")?;
            let host=app.state::<Host>();let mut engine=host.lock()?;
            let receipt=engine.store.forget_memory_evidence(&b.agent_id,id,string(&params,"evidenceId")?,params.get("evidenceRevision").and_then(Value::as_u64).ok_or("缺少记忆版本")?).map_err(|_|"无法更新记忆档案")?;
            host.memory.cancel_where(|task|task.binding_id==id);
            serde_json::to_value(receipt).map_err(|_|"记忆档案无效".into())
        },
        "memory.archive.activate"=>{
            expected(&b,&params)?;
            let service=string(&params,"serviceId")?;
            if !context.record.services.iter().any(|s|s.id==service&&s.kind==mewu_extension::ServiceKind::Memory) {return Err("插件未注册此记忆服务".into());}
            let selection_revision=params.get("bindingRevision").and_then(Value::as_u64).ok_or("缺少扩展绑定版本")?;
            live_store(app,context,|store|{
                let host=app.state::<Host>();let engine=host.lock()?;
                let current=engine.store.memory_binding(id).map_err(|_|"记忆档案不可用")?;
                expected(&current,&params)?;
                engine.store.memory_archive_route(&b.agent_id,id).map_err(|_|"记忆档案不可用")?;
                if !current.enabled || current.status!=mewu_core::BindingStatus::Ready {return Err("旧连接未就绪".into());}
                store.set_binding(&b.agent_id,"memory",Some(&context.record.manifest.id),Some(service),Some(selection_revision))
            })?;
            let extensions=app.state::<ExtensionHost>();extensions.cancel_agent(&b.agent_id);
            let bindings=extensions.store()?.bindings()?;
            let _=app.emit("extension-bindings",&bindings);
            Ok(json!({"ok":true}))
        },
        _=>Err("不支持的记忆档案操作".into())
    }
}
