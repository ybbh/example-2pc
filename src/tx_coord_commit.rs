use std::collections::HashMap;
use std::sync::Arc;

use rand::prelude::*;
use scupt_net::message_receiver_async::ReceiverAsync;
use scupt_net::message_sender_async::SenderAsync;
use scupt_net::notifier::Notifier;
use scupt_util::id::XID;
use scupt_util::message::Message;
use scupt_util::node_id::NID;
use scupt_util::res::Res;
use sedeve_kit::input;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use tracing::debug;

use crate::dtm_testing_msg::{DTMTesting, MTState};

use crate::rm_state::RMState;
use crate::tm_state::TMState;
use crate::tx_coord_event::TxCoordEvent;
use crate::tx_msg::TxMsg;
use crate::tx_msg_to_rm::MsgToRM;
use crate::tx_msg_to_tm::MsgToTM;
use crate::tx_rm::TxRM;
use crate::tx_tm::TxTM;

/// struct TxCoordCommit definition
pub struct TxCoordCommit {
    notify: Notifier,
    channel: Vec<Arc<dyn SenderAsync<TxMsg>>>,
    node_id: NID,
    tm_map: Arc<Mutex<HashMap<XID, Arc<TxTM>>>>,
    rm_map: Arc<Mutex<HashMap<XID, Arc<TxRM>>>>,
    event_receiver: Arc<Mutex<UnboundedReceiver<TxCoordEvent>>>,
    event_sender: UnboundedSender<TxCoordEvent>,
    auto_name:String,
} // struct TxCoordCommit definition end

/// TxCoordCommit implement
impl TxCoordCommit {
    pub fn new(
        auto_name:String,
        node_id: NID,
        sender: Arc<dyn SenderAsync<TxMsg>>,
        notify: Notifier,
    ) -> Self {
        let (s, r) = unbounded_channel();
        Self {
            notify,
            channel: vec![sender],
            node_id,
            tm_map: Default::default(),
            rm_map: Default::default(),
            event_receiver: Arc::new(Mutex::new(r)),
            event_sender: s,
            auto_name,
        }
    }



    pub async fn handle(&self) -> Res<()> {
        let mut r = self.event_receiver.lock().await;
        loop {
            let opt = r.recv().await;
            match opt {
                Some(e) => {
                    self.handle_event(e).await?;
                }
                None => {
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_event(&self, e: TxCoordEvent) -> Res<()> {
        match e {
            TxCoordEvent::TMEnd(xid) => {
                self.remove_tm(xid).await?;
            }
            TxCoordEvent::RMEnd(xid) => {
                self.remove_rm(xid).await?
            }
        }
        Ok(())
    }

    pub async fn incoming_message(&self, receiver: Arc<dyn ReceiverAsync<TxMsg>>) -> Res<()> {
        loop {
            let m = receiver.receive().await?;
            debug!("NODE receive message: {:?}", m);
            if self.notify.is_notified() {
                break;
            }
            let source = m.source();
            let dest = m.dest();
            let mc = m.clone();
            self.recv_msg(m.payload(), source, dest).await?;
            debug!("NODE receive message: {:?} ,done", mc);
        }
        Ok(())
    }

    async fn recv_msg(&self, message: TxMsg, source: NID, dest: NID) -> Res<()> {
        let _m = Message::new(message.clone(), source, dest);
        match message {
            TxMsg::TMMsg(m) => {
                input!(self.auto_name.as_str(), _m);
                // RM receive message from TM
                let tx = self.get_rm(m.xid).await?;
                tx.recv_msg(m.msg, source, dest).await?;
            }
            TxMsg::RMMsg(m) => {
                input!(self.auto_name.as_str(), _m);
                // TM receive message from RM
                let tx = self.get_tm(m.xid).await?;
                tx.recv_msg(m.msg, source, dest).await?;
            }
            TxMsg::DTMTesting(m) => {
                match &m {
                    DTMTesting::Restart(_nid) => {
                        input!(self.auto_name.as_str(), _m);
                        self.restart().await?;
                    }
                    DTMTesting::Setup(s)=> {
                        input!(self.auto_name.as_str(), _m);
                        self.setup(s.clone()).await?;
                    }
                    DTMTesting::Check(c) => {
                        input!(self.auto_name.as_str(), _m);
                        self.check(c.clone()).await?;
                    }
                    _ => {
                        input!(self.auto_name.as_str(), _m);
                        if let Some(xid) = &m.tm_xid() {
                            let tx = self.get_tm(xid.clone()).await?;
                            tx.recv_msg(MsgToTM::DTMTesting(m.clone()), source, dest).await?;
                        }
                        if let Some(xid) = &m.rm_xid() {
                            let tx = self.get_rm(xid.clone()).await?;
                            tx.recv_msg(MsgToRM::DTMTesting(m.clone()), source, dest).await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn get_tm(&self, xid: XID) -> Res<Arc<TxTM>> {
        let mut tm_map = self.tm_map.lock().await;
        let opt_tx = tm_map.get(&xid);
        let tx = match opt_tx {
            Some(tx) => {
                tx.clone()
            }
            None => {
                let tx =
                    TxTM::new(
                        self.auto_name.clone(),
                        self.node_id,
                        xid,
                        TMState::TMInvalid,
                        vec![],
                        self.get_channel().await?,
                        self.event_sender.clone(),
                        self.notify.clone(),
                    );
                tm_map.insert(xid, tx.clone());
                tx.clone()
            }
        };
        Ok(tx)
    }

    async fn get_rm(&self, xid: XID) -> Res<Arc<TxRM>> {
        let mut rm_map = self.rm_map.lock().await;
        let opt_tx = rm_map.get(&xid);
        let tx = match opt_tx {
            Some(tx) => {
                tx.clone()
            }
            None => {
                let tx =
                    TxRM::new(
                        self.auto_name.clone(),
                        self.node_id,
                        xid,
                        RMState::RMInvalid,
                        self.get_channel().await?,
                        self.event_sender.clone(),
                        self.notify.clone(),
                    );
                rm_map.insert(xid, tx.clone());
                tx.clone()
            }
        };
        Ok(tx)
    }

    async fn remove_rm(&self, xid: XID) -> Res<()> {
        let mut rm_map = self.rm_map.lock().await;
        let opt = rm_map.remove(&xid);
        if let Some(t) = opt {
            t.close();
        }
        Ok(())
    }

    async fn remove_tm(&self, xid: XID) -> Res<()> {
        let mut tm_map = self.tm_map.lock().await;
        let opt = tm_map.remove(&xid);
        if let Some(r) = opt {
            r.close();
        }
        Ok(())
    }


    async fn get_channel(&self) -> Res<Arc<dyn SenderAsync<TxMsg>>> {
        let mut rng = thread_rng();
        let opt = self.channel.choose(&mut rng);
        match opt {
            Some(a) => { Ok(a.clone()) }
            None => {
                panic!("no channel");
            }
        }
    }



    async fn restart(&self) -> Res<()> {
        {
            let mut _rm_map = self.rm_map.lock().await;
            for (_, t) in  _rm_map.iter() {
                t.restart().await?;
            }
        }

        {
            let mut _tm_map = self.tm_map.lock().await;
            for (_, t) in  _tm_map.iter() {
                t.restart().await?;
            }
        }
        Ok(())
    }

    async fn check(&self, state:MTState) -> Res<()> {
        for (x, _)  in state.rm_state.to_map() {
            let rm = self.get_rm(x).await?;
            rm.check_rm(state.clone()).await?;
        }

        for (x, _)  in state.tm_state.to_map() {
            let tm = self.get_tm(x).await?;
            tm.check_tm(state.clone()).await?;
        }

        Ok(())
    }

    async fn setup(&self, state:MTState) -> Res<()> {
        for (x, _)  in state.rm_state.to_map() {
            let rm = self.get_rm(x).await?;
            rm.setup_rm(state.clone()).await?;
        }

        for (x, _)  in state.tm_state.to_map() {
            let tm = self.get_tm(x).await?;
            tm.setup_tm(state.clone()).await?;
        }

        Ok(())
    }
} // impl TxCoordCommit end

