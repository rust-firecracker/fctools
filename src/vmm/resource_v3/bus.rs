use std::{
    future::Future,
    task::{Context, Poll},
};

pub trait Bus: 'static {
    type Client<Req: Send, Res: Send + Clone + 'static>: BusClient<Req, Res>;
    type Server<Req: Send, Res: Send + Clone + 'static>: BusServer<Req, Res>;

    fn new<Req: Send, Res: Send + Clone + 'static>() -> (Self::Client<Req, Res>, Self::Server<Req, Res>);
}

pub trait BusClient<Req: Send, Res: Send + Clone + 'static>: Send + Sync + Clone {
    type Incoming: BusIncoming<Res>;

    fn direct_try_read(&self) -> Option<Res>;

    fn request(&self, request: Req) -> Option<Self::Incoming>;

    fn request_and_read(&self, request: Req) -> impl Future<Output = Option<Res>> + Send {
        async { self.request(request)?.read().await }
    }
}

pub trait BusIncoming<Res: Send + Clone + 'static>: Send {
    fn read(self) -> impl Future<Output = Option<Res>> + Send;
}

pub trait BusServer<Req: Send, Res: Send + Clone + 'static>: Send + Unpin {
    type Outgoing: BusOutgoing<Res>;

    fn poll(&mut self, cx: &mut Context) -> Poll<Option<(Req, Self::Outgoing)>>;
}

pub trait BusOutgoing<Res: Send + Clone + 'static>: Send {
    fn write(self, response: Res) -> impl Future<Output = bool> + Send;
}

#[cfg(feature = "vmm-resource-default-bus")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-resource-default-bus")))]
pub mod default {
    use std::{
        pin::pin,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
        task::{Context, Poll},
    };

    use futures_channel::mpsc;
    use futures_util::StreamExt;

    use super::{Bus, BusClient, BusIncoming, BusOutgoing, BusServer};

    const DEFAULT_BUS_CAPACITY: usize = 100;

    pub struct DefaultBus;

    impl Bus for DefaultBus {
        type Client<Req: Send, Res: Send + Clone + 'static> = DefaultBusClient<Req, Res>;

        type Server<Req: Send, Res: Send + Clone + 'static> = DefaultBusServer<Req, Res>;

        fn new<Req: Send, Res: Send + Clone + 'static>() -> (Self::Client<Req, Res>, Self::Server<Req, Res>) {
            let (request_tx, request_rx) = mpsc::unbounded();
            let (response_tx, response_rx) = async_broadcast::broadcast(DEFAULT_BUS_CAPACITY);
            let connection_id_counter = Arc::new(AtomicU64::new(0));

            (
                DefaultBusClient {
                    request_tx,
                    response_rx,
                    connection_id_counter: connection_id_counter.clone(),
                },
                DefaultBusServer {
                    request_rx,
                    response_tx,
                    connection_id_counter,
                },
            )
        }
    }

    pub struct DefaultBusClient<Req, Res> {
        request_tx: mpsc::UnboundedSender<(u64, Req)>,
        response_rx: async_broadcast::Receiver<(u64, Res)>,
        connection_id_counter: Arc<AtomicU64>,
    }

    impl<Req: Send, Res: Send + Clone + 'static> BusClient<Req, Res> for DefaultBusClient<Req, Res> {
        type Incoming = DefaultBusIncoming<Res>;

        fn direct_try_read(&self) -> Option<Res> {
            match self.response_rx.new_receiver().try_recv() {
                Ok((_, response)) => Some(response),
                Err(_) => None,
            }
        }

        fn request(&self, request: Req) -> Option<Self::Incoming> {
            let connection_id = self.connection_id_counter.fetch_add(1, Ordering::Relaxed);
            self.request_tx.unbounded_send((connection_id, request)).ok()?;
            Some(DefaultBusIncoming {
                response_rx: self.response_rx.new_receiver(),
                connection_id,
            })
        }
    }

    impl<Req: Send, Res: Send + Clone> Clone for DefaultBusClient<Req, Res> {
        fn clone(&self) -> Self {
            Self {
                request_tx: self.request_tx.clone(),
                response_rx: self.response_rx.new_receiver(),
                connection_id_counter: self.connection_id_counter.clone(),
            }
        }
    }

    pub struct DefaultBusIncoming<Res> {
        response_rx: async_broadcast::Receiver<(u64, Res)>,
        connection_id: u64,
    }

    impl<Res: Send + Clone + 'static> BusIncoming<Res> for DefaultBusIncoming<Res> {
        async fn read(mut self) -> Option<Res> {
            while let Ok((id, response)) = pin!(self.response_rx.recv_direct()).await {
                if id == self.connection_id {
                    return Some(response);
                }
            }

            None
        }
    }

    pub struct DefaultBusServer<Req, Res> {
        request_rx: mpsc::UnboundedReceiver<(u64, Req)>,
        response_tx: async_broadcast::Sender<(u64, Res)>,
        connection_id_counter: Arc<AtomicU64>,
    }

    impl<Req: Send, Res: Send + Clone + 'static> BusServer<Req, Res> for DefaultBusServer<Req, Res> {
        type Outgoing = DefaultBusOutgoing<Res>;

        fn poll(&mut self, cx: &mut Context) -> Poll<Option<(Req, Self::Outgoing)>> {
            match self.request_rx.poll_next_unpin(cx) {
                Poll::Ready(Some((connection_id, request))) => {
                    let outgoing = DefaultBusOutgoing {
                        response_tx: self.response_tx.clone(),
                        connection_id,
                    };

                    Poll::Ready(Some((request, outgoing)))
                }
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    pub struct DefaultBusOutgoing<Res> {
        response_tx: async_broadcast::Sender<(u64, Res)>,
        connection_id: u64,
    }

    impl<Res: Send + Clone + 'static> BusOutgoing<Res> for DefaultBusOutgoing<Res> {
        async fn write(self, response: Res) -> bool {
            pin!(self.response_tx.broadcast_direct((self.connection_id, response)))
                .await
                .is_ok()
        }
    }
}
