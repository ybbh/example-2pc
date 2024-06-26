#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;
    use std::net::{IpAddr, SocketAddr};
    use std::sync::Arc;
    use std::thread;

    use scupt_net::es_option::ESServeOpt;
    use scupt_net::io_service::{IOService, IOServiceOpt};
    use scupt_net::io_service_async::IOServiceAsync;
    use scupt_net::notifier::Notifier;
    use scupt_net::task::spawn_local_task;
    use scupt_util::error_type::ET;
    use scupt_util::logger::logger_setup;
    use scupt_util::node_id::NID;
    use scupt_util::res::Res;
    use sedeve_kit::{auto_clear, auto_init};
    use sedeve_kit::dtm::action_incoming::ActionIncoming;
    use sedeve_kit::dtm::action_incoming_factory::ActionIncomingFactory;
    use sedeve_kit::dtm::dtm_player::{DTMPlayer, TestOption};
    use sedeve_kit::trace::trace_reader::TraceReader;
    use tokio::runtime::Builder;
    use tokio::task::LocalSet;
    use tracing::{debug, error};


    use crate::test_data_path::tests::test_data_path;
    use crate::tx_msg::TxMsg;
    use crate::tx_service::TxService;

    struct TestNode  {
        _coord_commit:Arc<TxService>,
        _service: Arc<dyn IOServiceAsync<TxMsg>>,
        join_handle : thread::JoinHandle<()>,
    }

    impl TestNode {
        fn start_node(
            auto_name:String,
            node_id: NID,
            name:String,
            node_addr: SocketAddr,
            notifier: Notifier,
        ) -> Res<Self> {
            debug!("run simulating {}", node_id);
            let opt = IOServiceOpt {
                num_message_receiver: 1,
                testing: true,
                sync_service: false,
                port_debug: None,
            };
            let s = IOService::<TxMsg>::new_async_service(
                    node_id.clone(), name,
                    opt, notifier.clone())?;
            let service = s;
            let receivers = service.receiver();
            let sink = service.default_sink();
            let sender = service.default_sender();
            let r = Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            let runtime = Arc::new(r);


            let _stop_notify_node = notifier.clone();
            let receiver = receivers[0].clone();
            let coord_commit = Arc::new(
                TxService::new(auto_name, node_id, sender, notifier.clone()));
            let ss = service.clone();

            let cc1 = coord_commit.clone();
            let cc3 = coord_commit.clone();
            let join_handle = thread::spawn(move ||{
                let ls = LocalSet::new();
                ls.spawn_local(async move {
                    let n1 = notifier.clone();
                    let _ = spawn_local_task(
                        n1,
                        format!("node_serve_{}", node_id).as_str(),
                        async move{
                            let r = sink.serve(node_addr, ESServeOpt::default()).await;
                            match r {
                                Ok(()) => {}
                                Err(e) => { error!("{}", e.to_string()) }
                            }
                        }
                    );
                    let n2 = notifier.clone();
                    let _ = spawn_local_task(
                        n2,
                        format!("message_loop_{}", node_id).as_str(),
                        async move {
                            let r = cc1.incoming(receiver).await;
                            match r {
                                Ok(()) => {}
                                Err(e) => { error!("{}", e.to_string()) }
                            }
                        }
                    );

                    let n3 = notifier.clone();
                    let _ = spawn_local_task(n3,
                    "handle",
                    async move {
                        cc3.handle().await?;
                        Ok::<(), ET>(())
                    });
                });


                ss.block_run(Some(ls), runtime);
            });
            Ok(Self {
                _coord_commit: coord_commit,
                _service: service,
                join_handle,
            })
        }

        fn join(self) {
            self.join_handle.join().unwrap();
        }
    }

    #[derive(Clone)]
    struct TestTxCoordCommit {
        node_id:NID,
        test_node:HashMap<NID, SocketAddr>,
        simulator_node:(NID, SocketAddr),
        auto_name:String,
    }

    impl TestTxCoordCommit {
        fn new(
            auto_name:String,
            test_node:HashMap<NID, SocketAddr>,
            simulator_node:(NID, SocketAddr)
        ) -> Self {
            Self {
                node_id: 1234,
                test_node,
                simulator_node,
                auto_name,
            }
        }

        fn run_testing(&self,  notifier:Notifier) -> Res<Vec<TestNode>> {
            let mut nodes = vec![];

            for (n, addr) in self.test_node.iter() {
                let name = format!("test_{}", n);
                let node = TestNode::start_node(
                    self.auto_name.clone(),
                    n.clone(),
                    name,
                    addr.clone(),
                    notifier.clone()
                )?;
                nodes.push(node)
            }
            Ok(nodes)
        }

    }

    struct TestContext {

        inner: TestTxCoordCommit,
    }

    impl TestContext {
        fn new(
            auto_name:String,
            test_node:HashMap<NID, SocketAddr>,
            simulator_node:(NID, SocketAddr)
        ) -> Self {
            Self {
                inner: TestTxCoordCommit::new(auto_name, test_node.clone(), simulator_node.clone())
            }
        }


        fn test_input_from_db(&self, db_path:String) -> Res<()> {
            let vec_incoming =
                TraceReader::read_trace(db_path)?;
            for (id, incoming) in vec_incoming.iter().enumerate() {
                debug!("run testing {}", id + 1);
                self.run_trace(incoming.clone())?;
            }
            Ok(())
        }

        fn test_input_from_json(&self, json_path:String) -> Res<()> {
            let incoming = ActionIncomingFactory::action_incoming_from_json_file(json_path)?;
            self.run_trace(incoming)?;
            Ok(())
        }

        fn run_trace(&self, incoming:Arc<dyn ActionIncoming>) -> Res<()> {
            let notifier = Notifier::new_with_name("run test".to_string());
            let notifier2 = notifier.clone();
            let player_node_id = self.inner.simulator_node.0;
            let player_addr = self.inner.simulator_node.1.clone();
            let peers = self.inner.test_node.clone();
            let action_incoming = incoming.clone();

            let node_id = self.inner.node_id.clone();
            let addr_str = player_addr.to_string();
            auto_init!(
                self.inner.auto_name.as_str(),
                node_id,
                player_node_id,
                addr_str.as_str()
            );
            let thread = thread::Builder::new().spawn(move ||{
                DTMPlayer::run_trace(
                    player_node_id,
                    player_addr,
                    peers,
                    action_incoming,
                    notifier.clone() ,
                    TestOption::default(),
                    move || {
                        notifier.notify_all();
                    }
                ).unwrap();
            }).unwrap();

            let nodes = self.inner.run_testing(
                notifier2.clone())?;

            thread.join().unwrap();
            for node in nodes {
                node.join();
            }
            auto_clear!(self.inner.auto_name.as_str());
            Ok(())
        }
    }


    pub fn test_2pc_dtm(
        auto_name:String,
        port_base:u16,
        num_node:u64,
        from_db_path:Result<String, String>) {
        logger_setup("debug");
        let mut test_node = HashMap::new();
        let simulator_node = (
                1000 as NID,
                SocketAddr::new(IpAddr::V4("127.0.0.1".parse().unwrap()), port_base)
            );
        for i in 1..=num_node {
            let addr = SocketAddr::new(IpAddr::V4("127.0.0.1".parse().unwrap()), port_base + i as u16);
            test_node.insert(i as NID, addr);
        }
        let ctx = TestContext::new(
            auto_name,
            test_node, simulator_node);
        if let Ok(p) = from_db_path {
            let path = test_data_path(p);
            let _ = ctx.test_input_from_db(path);
        } else  if let Err(p) = from_db_path {
            let path = test_data_path(p);
            let _ = ctx.test_input_from_json(path).unwrap();
        }
    }
}