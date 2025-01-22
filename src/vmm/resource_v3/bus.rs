use std::{
    future::Future,
    task::{Context, Poll},
};

pub trait Bus: 'static {
    type Client<Req: Send, Res: Send + Clone + 'static>: BusClient<Req, Res>;
    type Server<Req: Send, Res: Send + Clone + 'static>: BusServer<Req, Res>;

    fn new<Req: Send, Res: Send + Clone>() -> (Self::Client<Req, Res>, Self::Server<Req, Res>);
}

pub trait BusClient<Req: Send, Res: Send + Clone + 'static>: Send + Clone {
    fn start_request(&mut self, request: Req) -> bool;

    fn request(&mut self, request: Req) -> impl Future<Output = Option<Res>> + Send;
}

pub trait BusServer<Req: Send, Res: Send + Clone + 'static>: Send + Unpin {
    fn poll(&mut self, cx: &mut Context) -> Poll<Option<Req>>;

    fn response(&mut self, response: Res) -> impl Future<Output = bool> + Send + 'static;
}

pub mod default {
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll},
    };

    use futures_channel::mpsc;
    use futures_util::StreamExt;

    use super::{Bus, BusClient, BusServer};

    const DEFAULT_BUS_CAPACITY: usize = 25;

    pub struct DefaultBus;

    impl Bus for DefaultBus {
        type Client<Req: Send, Res: Send + Clone + 'static> = DefaultBusClient<Req, Res>;

        type Server<Req: Send, Res: Send + Clone + 'static> = DefaultBusServer<Req, Res>;

        fn new<Req: Send, Res: Send + Clone + 'static>() -> (Self::Client<Req, Res>, Self::Server<Req, Res>) {
            let (request_tx, request_rx) = mpsc::unbounded();
            let (response_tx, response_rx) = async_broadcast::broadcast(DEFAULT_BUS_CAPACITY);

            (
                DefaultBusClient {
                    request_tx,
                    response_rx,
                },
                DefaultBusServer {
                    request_rx,
                    response_tx,
                },
            )
        }
    }

    pub struct DefaultBusClient<Req, Res> {
        request_tx: mpsc::UnboundedSender<Req>,
        response_rx: async_broadcast::Receiver<Res>,
    }

    impl<Req: Send, Res: Send + Clone + 'static> BusClient<Req, Res> for DefaultBusClient<Req, Res> {
        fn start_request(&mut self, request: Req) -> bool {
            self.request_tx.unbounded_send(request).is_ok()
        }

        async fn request(&mut self, request: Req) -> Option<Res> {
            self.request_tx.unbounded_send(request).ok()?;
            pin!(self.response_rx.recv_direct()).await.ok()
        }
    }

    impl<Req: Send, Res: Send + Clone> Clone for DefaultBusClient<Req, Res> {
        fn clone(&self) -> Self {
            Self {
                request_tx: self.request_tx.clone(),
                response_rx: self.response_rx.new_receiver(),
            }
        }
    }

    pub struct DefaultBusServer<Req, Res> {
        request_rx: mpsc::UnboundedReceiver<Req>,
        response_tx: async_broadcast::Sender<Res>,
    }

    impl<Req: Send, Res: Send + Clone + 'static> BusServer<Req, Res> for DefaultBusServer<Req, Res> {
        fn poll(&mut self, cx: &mut Context) -> Poll<Option<Req>> {
            self.request_rx.poll_next_unpin(cx)
        }

        fn response(&mut self, response: Res) -> impl Future<Output = bool> + Send + 'static {
            let tx = self.response_tx.clone();
            async move { pin!(tx.broadcast_direct(response)).await.is_ok() }
        }
    }
}
