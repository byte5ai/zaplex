# Dockerfile for Linux Development

The Dockerfile in this directory defines a container that has all of the necessary tools installed to quickly get engineers up and running with building Warp on Linux.

This container is based on Debian Sid, Debian's unstable branch.  It ensures that you are running the latest versions of things like `mesa` (an open-source 3D graphics library, providing implementations of OpenGL and Vulkan).

## Prerequisites

You'll need to install:
* Docker (e.g.: Docker Desktop)
* XQuartz (download [here](https://www.xquartz.org/))
  * You'll want to enable iGLX (indirect GL extensions) for proper rendering by running `defaults write org.xquartz.X11 enable_iglx -bool true`; you can do this before you install XQuartz.
  * After installing XQuartz, run it, and enable "Allow connections from network clients" in the Security tab in its settings.  You'll need to quit and relaunch XQuartz after making this change.

## Setup

Run these commands from the repository root.

First, build the docker container image:

```bash
CONTAINER_NAME="warp-client-linux-dev"
docker build -f docker/linux-dev/Dockerfile -t "$CONTAINER_NAME" .
```

Next, run the container:

```bash
# The path to the source code directory that you want to mount in the
# container. This can be the Zaplex repository or some parent
# directory of your choice.
LOCAL_PATH="/Users/$USER/src"

# Run the image as a detached container and mount only the source tree.
# Host SSH keys and cloud credentials deliberately stay outside the container.
docker run --detach \
  --name "$CONTAINER_NAME" \
  --volume "$LOCAL_PATH:/src" \
  "$CONTAINER_NAME"
```

## Usage

Every time you start XQuartz, you'll need to run this once in order for programs running in the container to connect to it:

```bash
xhost +localhost
```

Enter the container with `docker exec`; it does not run an SSH server and publishes no login port:

```bash
docker exec --interactive --tty "$CONTAINER_NAME" zsh
cd /src
cargo run --features fast_dev
```

If a development task needs GitHub or cloud access, authenticate inside the running container
for that run. Do not mount `~/.ssh` or `~/.config/gcloud` into the container.

It's possible you'll run into some odd errors while trying to compile Warp; if so, just keep rerunning the cargo command and it should work eventually.
