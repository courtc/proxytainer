use docker_api::opts::{ContainerFilter, ContainerListOpts, ContainerStopOpts};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{ Duration, Instant },
};
use anyhow::Result;
use log::{debug, info};

pub enum DockerMessageType {
    ContainerRequire,
    ContainerPoke,
}

pub struct DockerMessage {
    pub message_type: DockerMessageType,
    pub reply_to: Option<oneshot::Sender<Result<()>>>,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum DockerManagerState {
    Idle,
    Starting,
    Running,
    Stopping,
}

// Idle
//      Starting
//          Running
//      Stopping

struct DockerManagerService {
    docker: docker_api::Docker,
    containers: Vec<String>,
    poll_period: Duration,
    idle_duration: Duration,
    poke_time: Instant,
    group: String,
    pending_replies: Vec<(DockerManagerState, oneshot::Sender<Result<()>>)>,
    pending_restart: bool,
    receiver: mpsc::Receiver<DockerMessage>,
    state: DockerManagerState,
}

impl DockerManagerService {
    async fn run(&mut self) {
        let container_list_opts = ContainerListOpts::builder()
            .all(true)
            .filter(vec![
                ContainerFilter::Label("proxytainer.group".into(), self.group.clone()),
            ]).build();

        self.containers.extend(
            self.docker.containers().list(&container_list_opts).await
                .expect("Failed to list containers")
                .into_iter()
                .filter_map(|container| container.id)
                .inspect(|id| info!("Found container: {}", &id[..12]))
        );

        if self.containers.is_empty() {
            panic!("No containers found");
        }

        let max_poll = Duration::from_secs(5);

        self.poll_container().await;
        loop {
            while let Ok(opt_msg) = tokio::time::timeout(self.poll_period, self.receiver.recv()).await {
                if let Some(msg) = opt_msg {
                    self.handle_message(msg).await;
                } else {
                    debug!("Queue closed");
                    return;
                }
            }
            self.poll_container().await;
            if self.poll_period < max_poll {
                self.poll_period = (self.poll_period + self.poll_period / 2).min(max_poll);
            }
        }
    }

    fn reset_poll_period(&mut self) {
        self.poll_period = Duration::from_millis(125);
    }

    async fn poll_container(&mut self) {
        use DockerManagerState::*;

        let mut new_states = Vec::new();
        for id in &self.containers {
            let container = self.docker.containers().get(id);
            let data = container.inspect().await
                .expect("Failed to inspect container");
            //println!("Container state: {:?}", data);
            let Some(container_state) = &data.state else {
                continue;
            };
            let state = match container_state.status.as_deref() {
                Some("running") => {
                    if let Some(health) = &container_state.health {
                        match health.status.as_deref() {
                            Some("healthy" | "none") => Running,
                            Some("starting") => Starting,
                            _ => Starting,
                        }
                    } else {
                        Running
                    }
                },
                Some("restarting") => Starting,
                _ => Idle,
            };

            if !new_states.contains(&state) {
                new_states.push(state);
            }
        }

        if new_states.len() == 1 && new_states[0] != self.state {
            let state = new_states[0];
            debug!("Received state change from {:?} to {:?}", self.state, state);
            match (self.state, state) {
                (Idle, Running) => {
                    self.on_state_change(Starting);
                    self.on_state_change(state);
                },
                (Starting, Running | Stopping) |
                (Stopping, Idle) |
                (Running, Stopping) |
                (Idle, Starting) => {
                    self.on_state_change(state);
                },
                (Stopping, Starting) => {
                    self.on_state_change(Idle);
                    self.on_state_change(state);
                }
                (Running, Idle) |
                (Starting, Idle) => {
                    self.on_state_change(Stopping);
                    self.on_state_change(state);
                },
                (Idle, Stopping) |
                (Stopping, Running) |
                (Running, Starting) => {},
                _ => unreachable!(),
            }
        }

        if self.state == Running {
            let now = Instant::now();
            debug!("Idle for {} seconds", (now - self.poke_time).as_secs());
            if now - self.poke_time > self.idle_duration {
                self.stop_container().await;
            }
        }

        if self.state == Idle && self.pending_restart {
            self.start_container().await;
            self.pending_restart = false;
        }
    }

    async fn start_container(&mut self) {
        info!("Starting container");
        self.on_state_change(DockerManagerState::Starting);
        for id in &self.containers {
            let container = self.docker.containers().get(id);
            container.start().await
                .expect("Failed to start container");
        }
    }

    async fn stop_container(&mut self) {
        info!("Stopping container");
        self.on_state_change(DockerManagerState::Stopping);
        for id in &self.containers {
            let container = self.docker.containers().get(id);
            container.stop(&ContainerStopOpts::builder().build()).await
                .expect("Failed to stop container");
        }
    }

    fn on_state_change(&mut self, state: DockerManagerState) {
        use DockerManagerState::*;

        info!("State change ({:?} => {:?})", self.state, state);
        let mut old_state = state;
        std::mem::swap(&mut self.state, &mut old_state);

        let mut i = 0;
        while i < self.pending_replies.len() {
            if self.pending_replies[i].0 == self.state {
                let (_, reply_to) = self.pending_replies.remove(i);
                let _ = reply_to.send(Ok(()));
            } else {
                i += 1;
            }
        }

        match (old_state, &self.state) {
            (_, Idle) => { },
            (_, Starting) => {
                self.reset_poll_period();
            },
            (_, Running) => {
                self.poke_time = Instant::now();
            },
            (_, Stopping) => {
                self.reset_poll_period();
            },
        }
    }

    fn queue_response(&mut self, reply_to: Option<oneshot::Sender<Result<()>>>, state: DockerManagerState) {
        if let Some(reply_to) = reply_to {
            if state == self.state {
                let _ = reply_to.send(Ok(()));
            } else {
                self.pending_replies.push((state, reply_to));
            }
        }
    }

    async fn handle_message(&mut self, msg: DockerMessage) {
        use DockerManagerState::*;
        use DockerMessageType::*;
        match (&self.state, msg.message_type) {
            (Idle, ContainerRequire) => {
                self.queue_response(msg.reply_to, Running);
                self.start_container().await;
            },
            (Starting | Running, ContainerRequire) => {
                // Already starting/started
                self.queue_response(msg.reply_to, Running);
            },
            (Stopping, ContainerRequire) => {
                self.pending_restart = true;
                self.queue_response(msg.reply_to, Running);
            },
            (_, ContainerPoke) => {
                self.poke_time = Instant::now();
                self.queue_response(msg.reply_to, self.state);
            },
        }
    }
}

pub struct DockerManager {
    pub sender: mpsc::Sender<DockerMessage>,
    handle: JoinHandle<()>,
}

impl DockerManager {
    pub fn new(group: String, idle: u64) -> Result<Self> {
        let (send, recv) = mpsc::channel(8);
        let mut service = DockerManagerService{
            state: DockerManagerState::Starting,
            //docker: docker_api::Docker::new("unix:///var/run/docker.sock")?,
            docker: docker_api::Docker::unix("/var/run/docker.sock"),
            pending_replies: Vec::new(),
            pending_restart: false,
            poke_time: Instant::now(),
            containers: Vec::new(),
            poll_period: Duration::from_millis(125),
            idle_duration: Duration::from_secs(idle),
            group,
            receiver: recv,
        };
        let handle = tokio::spawn(async move {
            service.run().await
        });
        Ok(Self { sender: send, handle })
    }

    pub async fn wait_healthy(&self) -> Result<()> {
        let (send, recv) = oneshot::channel();
        let msg = DockerMessage {
            message_type: DockerMessageType::ContainerRequire,
            reply_to: Some(send),
        };

        let _ = self.sender.send(msg).await;
        recv.await.expect("task killed")
    }
}
