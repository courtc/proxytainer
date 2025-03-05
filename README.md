# Proxytainer

Control containers via a transparent TCP proxy.

## Overview

This tool allows you to control containers based on proxied TCP activity. The tool listens on a TCP port, and forwards any connection and data to a specified server. On TCP connection or traffic the container is started (if stopped). After a configurable idle duration, the container is stopped.

The tool will hold TCP connections idle while waiting for containers to start, and will resume activity once they are running and healthy (if health checks are enabled and the container has a health check).

Other than TCP, this tool is protocol agnostic, though it may not work in cases where timing is critical.

## Features

- Container health support
- Configurable idle duration
- Transparent data proxying
- Container start on connect

## Configuration

Most of the configuration is done via command line arguments. There are two exceptions:
 - The proxied containers must specify their group via a docker label `proxytainer.group=xxx`. This must match the `--group` argument of proxytainer.
 - I use [env_logger](https://github.com/rust-cli/env_logger) for logging, and set the default log level to `info`. You can change the log level via the `RUST_LOG` environment variable. An example is provided in the `docker-compose.yml` file.

```
Usage: proxytainer [OPTIONS] --port <PORT> --host <HOST> --group <GROUP>

Options:
  -p, --port <PORT>    Port to listen on
      --host <HOST>    Server address
  -g, --group <GROUP>  Container group name
      --idle <IDLE>    Container idle time (seconds) [default: 300]
      --no-health      Disable docker health check
  -h, --help           Print help
  -V, --version        Print version
```

## Example

```yaml
  myserver:
    command: myserver --port 5000  # listen on localhost:5000
    network_mode: service:proxytainer
    labels:
      - proxytainer.group=mygroup # group must match --group of proxytainer
  proxytainer:
    build: ./proxytainer
    # our setup: 5000(external) -> 5001(proxytainer) -> 5000(myserver)
    # listen on :5001, connect to localhost:5000
    command: --port 5001 --host localhost:5000 --group mygroup --idle 600
    ports:
      - 5000:5001
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro # required
```

You can also try out the example `docker-compose.yml` provided:
1. Clone this repository
    ```sh
    git clone https://github.com/courtc/proxytainer
    cd proxytainer
    ```
2. Run the example
    ```sh
    docker-compose up --build
    ```
    This will start a proxytainer container and a whoami container. The proxytainer container will listen on port 9090 and proxy traffic to the whoami container. The proxytainer container will stop the whoami container after 30 seconds of inactivity.
3. Navigate to the container address
    ```
    $ time curl -s localhost:9090 | head -n 1
    Hostname: 816f74b1f006

    real    0m0.010s
    ```
4. Wait until the docker compose logs say `State change (Stopping => Idle)`. The whoami container is now stopped. You can see this with `docker ps -a`.
5. Again, navigate to the container address. This will start the whoami container and pass the connection through to the container. No lost network traffic.
    ```
    $ time curl -s localhost:9090 | head -n 1
    Hostname: 816f74b1f006

    real    0m0.287s
    ```
6. You can now ctrl-c the docker-compose process to stop the containers. Don't forget to also clean up the containers:
    ```sh
    docker-compose down
    ```

## Future Work

This project is basically still in alpha state, so there's lots of room for improvement. That being said, it works for my purposes, so I'm not likely to be adding new features until I need them. Here are some likely candidates for improvement:
- UDP support
- Traffic thresholds (like [Lazytainer](https://github.com/vmorganp/Lazytainer))
- Proper group management
- Better error handling

## Inspiration

This project is heavily inspired by [Lazytainer](https://github.com/vmorganp/Lazytainer). If you're interested in UDP support or more control over network activity, check that project out instead. Maybe it'll meet your needs.
