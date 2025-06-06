use crate::{
    containers::{HashMap, Storage, World},
    gametypes::{MAX_SOCKET_PLAYERS, Result},
    socket::{Client, ClientState},
};
use log::{trace, warn};
use mio::{Events, Poll, net::TcpListener};
use std::{cell::RefCell, collections::VecDeque, io, sync::Arc, time::Duration};

pub const SERVER: mio::Token = mio::Token(0);
pub const TLS_SERVER: mio::Token = mio::Token(1);

pub struct Server {
    pub listener: TcpListener,
    pub tls_listener: TcpListener,
    pub clients: HashMap<mio::Token, RefCell<Client>>,
    pub tokens: VecDeque<mio::Token>,
    pub tls_config: Arc<rustls::ServerConfig>,
}

impl Server {
    #[inline]
    pub fn new(
        poll: &mut Poll,
        addr: &str,
        tls_addr: &str,
        max: usize,
        cfg: Arc<rustls::ServerConfig>,
    ) -> Result<Server> {
        /* Create a bag of unique tokens. */
        let mut tokens = VecDeque::with_capacity(max);

        for i in 2..max {
            tokens.push_back(mio::Token(i));
        }

        /* Set up the TCP listener. */
        let addr = addr.parse()?;
        let mut listener = TcpListener::bind(addr)?;

        let tls_addr = tls_addr.parse()?;
        let mut tls_listener = TcpListener::bind(tls_addr)?;

        poll.registry()
            .register(&mut listener, SERVER, mio::Interest::READABLE)?;
        poll.registry()
            .register(&mut tls_listener, TLS_SERVER, mio::Interest::READABLE)?;

        Ok(Server {
            listener,
            tls_listener,
            clients: HashMap::default(),
            tokens,
            tls_config: cfg,
        })
    }

    pub fn accept(&mut self, storage: &Storage, is_tls: bool) -> Result<()> {
        /* Wait for a new connection to accept and try to grab a token from the bag. */
        loop {
            let (stream, addr) = match if is_tls {
                self.tls_listener.accept()
            } else {
                self.listener.accept()
            } {
                Ok((stream, addr)) => (stream, addr),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    trace!("listener.accept error: {}", e);
                    return Err(e.into());
                }
            };

            if !is_tls {
                stream.set_nodelay(true)?;
            }

            if let Some(token) = self.tokens.pop_front() {
                if self.clients.len() + 1 >= MAX_SOCKET_PLAYERS {
                    warn!(
                        "Server is full. has reached MAX_SOCKET_PLAYERS: {} ",
                        MAX_SOCKET_PLAYERS
                    );
                    drop(stream);
                    return Ok(());
                }

                let tls_conn = if is_tls {
                    Some(rustls::ServerConnection::new(Arc::clone(&self.tls_config))?)
                } else {
                    None
                };

                // Lets make the Client to handle hwo we send packets.
                let mut client = Client::new(stream, token, tls_conn, addr.to_string())?;
                //client.poll_state.add(crate::socket::PollState::Write);
                //Register the Poll to the client for recv and Sending
                client.register(&storage.poll.borrow_mut())?;

                // insert client into handled list.
                self.clients.insert(token, RefCell::new(client));
            } else {
                warn!("listener.accept No tokens left to give out.");
                drop(stream);
            }
        }
        Ok(())
    }

    #[inline]
    pub fn remove(&mut self, token: mio::Token) {
        /* If the token is valid, let's remove the connection and add the token back to the bag. */
        if self.clients.contains_key(&token) {
            self.clients.remove(&token);
            self.tokens.push_front(token);
        }
    }
}

pub fn poll_events(world: &mut World, storage: &Storage) -> Result<()> {
    let mut events = Events::with_capacity(1024);

    storage
        .poll
        .borrow_mut()
        .poll(&mut events, Some(Duration::from_millis(0)))?;

    for event in events.iter() {
        match event.token() {
            SERVER => {
                storage.server.borrow_mut().accept(storage, false)?;
                storage.poll.borrow_mut().registry().reregister(
                    &mut storage.server.borrow_mut().listener,
                    SERVER,
                    mio::Interest::READABLE,
                )?;
            }
            TLS_SERVER => {
                storage.server.borrow_mut().accept(storage, true)?;
                storage.poll.borrow_mut().registry().reregister(
                    &mut storage.server.borrow_mut().tls_listener,
                    TLS_SERVER,
                    mio::Interest::READABLE,
                )?;
            }
            token => {
                let mut server = storage.server.borrow_mut();
                let state = if let Some(a) = server.clients.get(&token) {
                    a.borrow_mut().process(event, world, storage)?;
                    a.borrow().state
                } else {
                    trace!("a token no longer exists within clients.");
                    ClientState::Closed
                };

                if state == ClientState::Closed {
                    server.remove(token);
                };
            }
        }
    }

    Ok(())
}
