use super::*;
use std::collections::HashMap;
use futures::{Future, BoxFuture};
use futures::future::{join_all, JoinAll};

#[derive(Clone)]
pub struct Room<T, R>
    where T: Send + 'static,
          R: Send + 'static
{
    clients: HashMap<ClientId, Client<T, R>>,
}

impl<T, R> Room<T, R>
    where T: Send + 'static,
          R: Send + 'static
{
    pub fn new(clients: Vec<Client<T, R>>) -> Room<T, R> {
        let clients = clients.into_iter().map(|c| (c.id(), c)).collect();
        Room { clients: clients }
    }

    pub fn client_ids(&self) -> Vec<ClientId> {
        self.clients.keys().cloned().collect()
    }

    pub fn client_names(&self) -> Vec<Option<String>> {
        self.clients.values().map(|c| c.name()).collect()
    }

    pub fn into_clients(self) -> Vec<Client<T, R>> {
        self.clients.into_iter().map(|(_, c)| c).collect()
    }

    // @TODO: Exists only for `Client::join`. When RFC1422 is stable, make this `pub(super)`.
    #[doc(hidden)]
    pub fn insert(&mut self, client: Client<T, R>) -> bool {
        if self.contains(&client.id()) {
            return false;
        }
        self.clients.insert(client.id(), client);
        true
    }

    pub fn remove(&mut self, id: &ClientId) -> Option<Client<T, R>> {
        self.clients.remove(id)
    }

    pub fn contains(&self, id: &ClientId) -> bool {
        self.clients.contains_key(id)
    }

    pub fn name_of(&self, id: &ClientId) -> Option<String> {
        self.clients.get(id).and_then(|c| c.name())
    }

    // *Copies* filtered clients into another Room.
    pub fn filter<F>(&self, mut f: F) -> Room<T, R>
        where F: FnMut(&Client<T, R>) -> bool,
              T: Clone,
              R: Clone
    {
        let filtered_client_clones = self.clients.values().filter(|c| f(c)).cloned().collect();
        Room::new(filtered_client_clones)
    }

    fn client_mut(&mut self, id: &ClientId) -> Option<&mut Client<T, R>> {
        self.clients.get_mut(id)
    }

    fn communicate_with_all_clients<F, G>(&mut self, f: F) -> JoinAll<Vec<G>>
        where F: FnMut(&mut Client<T, R>) -> G,
              G: Future
    {
        join_all(self.clients.values_mut().map(f).collect::<Vec<_>>())
    }

    pub fn broadcast(&mut self, msg: T) -> BoxFuture<<Self as Communicator>::Status, ()>
        where T: Clone
    {
        self.communicate_with_all_clients(|client| client.transmit(msg.clone()))
            .map(|results| results.into_iter().collect())
            .boxed()
    }
}

impl<T, R> Default for Room<T, R>
    where T: Send + 'static,
          R: Send + 'static
{
    fn default() -> Room<T, R> {
        Room { clients: HashMap::new() }
    }
}

impl<T, R> Communicator for Room<T, R>
    where T: Send + 'static,
          R: Send + 'static
{
    type Transmit = HashMap<ClientId, T>;
    type Receive = (HashMap<ClientId, ClientStatus>, HashMap<ClientId, R>);
    type Status = HashMap<ClientId, ClientStatus>;
    type Error = ();

    fn transmit(&mut self, msgs: Self::Transmit) -> BoxFuture<Self::Status, ()> {
        let client_futures = msgs.into_iter()
            .filter_map(|(id, msg)| self.client_mut(&id).map(|client| client.transmit(msg)))
            .collect::<Vec<_>>();
        join_all(client_futures).map(|results| results.into_iter().collect()).boxed()
    }

    fn receive(&mut self, timeout: ClientTimeout) -> BoxFuture<Self::Receive, ()> {
        self.communicate_with_all_clients(|client| client.receive(timeout))
            .map(|results| {
                let mut statuses = HashMap::new();
                let mut msgs = HashMap::new();
                for (id, status, msg) in results {
                    statuses.insert(id, status);
                    msg.and_then(|msg| msgs.insert(id, msg));
                }
                (statuses, msgs)
            })
            .boxed()
    }

    fn status(&mut self) -> BoxFuture<Self::Status, ()> {
        self.communicate_with_all_clients(|client| client.status())
            .map(|results| results.into_iter().collect())
            .boxed()
    }

    fn close(&mut self) -> BoxFuture<Self::Status, ()> {
        self.communicate_with_all_clients(|client| client.close())
            .map(|results| results.into_iter().collect())
            .boxed()
    }
}

pub trait Broadcasting {
    type M;
    type Status;

    fn broadcast(&mut self, msg: Self::M) -> BoxFuture<(Self::Status, Self::Status), ()>;
}

impl<T, R> Broadcasting for (Room<T, R>, Room<T, R>)
    where T: Clone + Send + 'static,
          R: Send + 'static
{
    type M = T;
    type Status = <Room<T, R> as Communicator>::Status;

    fn broadcast(&mut self, msg: T) -> BoxFuture<(Self::Status, Self::Status), ()> {
        self.0.broadcast(msg.clone()).join(self.1.broadcast(msg)).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test::*;
    use futures::Stream;

    #[test]
    fn can_transmit() {
        let (rx0, client0) = mock_client_channelled();
        let mut client0_rx = rx0.wait().peekable();
        let client0_id = client0.id();

        let (rx1, client1) = mock_client_channelled();
        let mut client1_rx = rx1.wait().peekable();
        let client1_id = client1.id();

        let mut room = Room::new(vec![client0, client1]);

        let mut msgs = HashMap::new();
        msgs.insert(client0_id, TinyMsg::A);
        msgs.insert(client1_id, TinyMsg::B("entropy".to_string()));
        room.transmit(msgs).wait().unwrap();
        match (client0_rx.next(), client1_rx.next()) {
            (Some(Ok(_)), Some(Ok(_))) => {}
            _ => assert!(false),
        }
    }
}