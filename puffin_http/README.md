# puffin_http

[![Embark](https://img.shields.io/badge/embark-open%20source-blueviolet.svg)](https://embark.dev)
[![Embark](https://img.shields.io/badge/discord-ark-%237289da.svg?logo=discord)](https://discord.gg/dAuKfZS)
[![Crates.io](https://img.shields.io/crates/v/puffin_http.svg)](https://crates.io/crates/puffin_http)
[![Docs](https://docs.rs/puffin_http/badge.svg)](https://docs.rs/puffin_http)

A HTTP server/client for communicating [`puffin`](https://github.com/EmbarkStudios/puffin) profiling events.

You can view them using [`puffin_viewer`](https://github.com/EmbarkStudios/puffin/tree/main/puffin_viewer).

# Architecture
## server
Listens for incoming connections and streams them puffin profiler data.
- struct Server
 - sink_id:
 - join_handle: **puffin-server** thread handle
 - num_clients: shared 'nb clients'
 - fn new() : 
 	- create channel for get frame info
 	- create PuffinServerImpl object to use in **puffin-server** thread
 	- create thread **puffin-server** for lisen for client and send frame to client
	
- struct Client
 - client_addr
 - packet_tx: channel sender
 - join_handle: **puffin-server-client** thread handle

- struct PuffinServerImpl
 - tcp_listener: tcp listener of clients
 - clients: list of **Client** object
 - num_clients: shared 'nb clients'
 - fn accept_new_clients :
  - loop on *tcp_listener*
  - create thread *puffin-server-client* when client tried to connect
  - and create **Client** object to allow PuffinSever to interract with client.
 - fn send :
  - send frame data by channel to clients thread
  
- fn client_loop :
 - send frame data to client by socket

## client
- TODO
