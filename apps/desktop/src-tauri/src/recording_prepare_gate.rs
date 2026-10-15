// SPDX-License-Identifier: MPL-2.0
//! Phase ordering shared by the real host supervisor and its recording worker.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Phase{Consent,Hidden,Frame,Published,Canceled}
#[derive(Debug,PartialEq,Eq)]
pub(crate) enum PreparationOutcome{Ready,Canceled}
/// Existing preparation only: cancellation is explicit host lifecycle evidence,
/// never an error-text match and never permission to admit new work.
pub(crate) fn preparation_step(mut canceled:impl FnMut()->Result<bool,String>,
    operation:impl FnOnce()->Result<(),String>)->Result<PreparationOutcome,String>{
    if canceled()?{return Ok(PreparationOutcome::Canceled)}
    let result=operation();
    if canceled()?{return Ok(PreparationOutcome::Canceled)}
    result.map(|_|PreparationOutcome::Ready)
}
pub(super) struct PreparationGate{nonce:String,phase:Phase}
impl PreparationGate{
    pub(super) fn new(nonce:&str)->Self{Self{nonce:nonce.into(),phase:Phase::Consent}}
    fn advance(&mut self,nonce:&str,from:Phase,to:Phase)->Result<(),String>{
        if self.nonce!=nonce||self.phase!=from{return Err("录屏准备回执无效".into())}
        self.phase=to;Ok(())
    }
    pub(super) fn consented(&mut self,nonce:&str)->Result<(),String>{self.advance(nonce,Phase::Consent,Phase::Hidden)}
    pub(super) fn hidden(&mut self,nonce:&str)->Result<(),String>{self.advance(nonce,Phase::Hidden,Phase::Frame)}
    pub(super) fn prepared(&mut self,nonce:&str)->Result<(),String>{self.advance(nonce,Phase::Frame,Phase::Published)}
    pub(super) fn cancel(&mut self){self.phase=Phase::Canceled;}
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn fresh_frame_requires_matching_consent_then_hidden_ack(){
        let mut g=PreparationGate::new("current");assert!(g.prepared("current").is_err());assert!(g.hidden("current").is_err());
        g.consented("current").unwrap();assert!(g.prepared("current").is_err());g.hidden("current").unwrap();g.prepared("current").unwrap();
    }
    #[test]fn stale_or_replayed_replies_never_advance_current_owner(){
        let mut g=PreparationGate::new("current");assert!(g.consented("old").is_err());g.consented("current").unwrap();
        assert!(g.consented("current").is_err());assert!(g.hidden("old").is_err());g.hidden("current").unwrap();
        assert!(g.hidden("current").is_err());assert!(g.prepared("old").is_err());g.prepared("current").unwrap();assert!(g.prepared("current").is_err());
    }
    #[test]fn cancellation_at_any_preparation_boundary_cannot_adopt_a_frame(){
        for boundary in 0..=2{let mut g=PreparationGate::new("current");
            if boundary>=1{g.consented("current").unwrap();}if boundary>=2{g.hidden("current").unwrap();}
            g.cancel();assert!(g.consented("current").is_err());assert!(g.hidden("current").is_err());assert!(g.prepared("current").is_err());
        }
    }
    #[test]fn exit_reserved_after_empty_control_before_consent_skips_hide_and_ack(){
        // The supervisor previously observed no control. The actual callback
        // must recheck current lifecycle evidence rather than trust that poll.
        let canceled=std::cell::Cell::new(false);let control:Option<()>=None;assert!(control.is_none());
        canceled.set(true);
        let result=preparation_step(||Ok(canceled.get()),||panic!("canceled consent must not hide or acknowledge"));
        assert_eq!(result.unwrap(),PreparationOutcome::Canceled);
    }
    #[test]fn exit_during_hidden_barrier_settles_as_canceled_not_a_finished_error(){
        let sticky_cancel=std::cell::Cell::new(false);
        let result=preparation_step(||Ok(sticky_cancel.get()),||{
            // Native check observed reservation while awaiting GTK unmap. Its
            // exact lease marks cancel even if the exit attempt later aborts.
            sticky_cancel.set(true);Err("existing-work admission closed".into())
        });
        assert_eq!(result.unwrap(),PreparationOutcome::Canceled);
    }
    #[test]fn real_preparation_error_without_explicit_cancellation_is_preserved(){
        for error in ["native GTK unmap failure","Portal transport failure","existing-work admission closed"]{
            assert_eq!(preparation_step(||Ok(false),||Err(error.into())),Err(error.into()));
        }
    }
}
