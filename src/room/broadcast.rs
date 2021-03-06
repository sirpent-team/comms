use std::hash::Hash;
use std::collections::HashSet;
use futures::{Future, Sink, Stream, Poll, Async, AsyncSink};
use super::*;

pub struct Broadcast<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkItem: Clone
{
    room: Option<Room<I, C>>,
    msg: C::SinkItem,
    start_send_list: HashSet<I>,
    poll_complete_list: Vec<I>,
}

impl<I, C> Broadcast<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkItem: Clone
{
    #[doc(hidden)]
    pub fn new(room: Room<I, C>, msg: C::SinkItem, ids: HashSet<I>) -> Broadcast<I, C> {
        Broadcast {
            room: Some(room),
            msg: msg,
            start_send_list: ids,
            poll_complete_list: vec![],
        }
    }

    pub fn into_inner(mut self) -> Room<I, C> {
        self.room.take().unwrap()
    }
}

impl<I, C> Future for Broadcast<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkItem: Clone
{
    type Item = Room<I, C>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut room = self.room.take().unwrap();

        let start_send_list = self.start_send_list.drain().collect::<Vec<_>>();
        for id in start_send_list {
            let ready_client = match room.client_mut(&id) {
                Some(ready_client) => ready_client,
                None => continue,
            };
            match ready_client.start_send(self.msg.clone()) {
                Ok(AsyncSink::NotReady(_)) => {
                    self.start_send_list.insert(id);
                }
                Ok(AsyncSink::Ready) => {
                    self.poll_complete_list.push(id);
                }
                Err(_) => {}
            }
        }

        let poll_complete_list = self.poll_complete_list.drain(..).collect::<Vec<_>>();
        for id in poll_complete_list {
            let ready_client = match room.client_mut(&id) {
                Some(ready_client) => ready_client,
                None => continue,
            };
            match ready_client.poll_complete() {
                Ok(Async::NotReady) => {
                    self.poll_complete_list.push(id);
                }
                Ok(Async::Ready(())) | Err(_) => {}
            }
        }

        if self.start_send_list.is_empty() && self.poll_complete_list.is_empty() {
            Ok(Async::Ready(room))
        } else {
            self.room = Some(room);
            Ok(Async::NotReady)
        }
    }
}
