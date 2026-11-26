# Relay Map on Kubernetes

Runs the hub in a cluster and serves it through an Envoy Gateway with a
wildcard certificate. The manifests here are generic; your overlay names your
computers and hostname and stays out of this repository.

## How data gets in

The hub in the pod collects nothing locally (`local = false`). It reaches every
computer over ssh, exactly as a desktop hub reaches remote ones: it copies the
matching collector from `/app/bin` to `~/.cache/relay-map/` there and streams
`relay-map collect`. The image ships Linux (x86_64, aarch64) and macOS (Apple
silicon) collectors.

So each computer needs:

- an address the pod can reach (pods leave through the node, so a VPN address
  the node routes to works);
- the hub's public key in `~/.ssh/authorized_keys`. Limit it, for example
  `restrict,from="<node address as the computer sees it>" ssh-ed25519 ... relay-map-k8s`
  (`restrict` turns off forwarding and terminals; the collector needs neither);
- its host key in the overlay's `known_hosts` (strict checking is on).

## Broadcast

The container has no agent-relay of its own. `relay-send-via-ssh.py` stands in
for `relay.py`: it runs the same `relay send` with the agent-relay installed on
the computer named by `broadcast_via`, over ssh, so delivery is what that
computer would do itself. Leave `broadcast_via` out to turn broadcast off.

## Access

`securitypolicy.yaml` puts HTTP basic auth in front of the route (the gateway
checks it, the WebSocket inherits it). Create the password file:

```sh
pw=$(openssl rand -base64 18)
printf 'operator:{SHA}%s\n' "$(printf %s "$pw" | openssl dgst -binary -sha1 | openssl base64)" > htpasswd
kubectl -n relay-map create secret generic relay-map-htpasswd --from-file=.htpasswd=htpasswd
```

## Deploy

```sh
deploy/k8s/build-image.sh <ssh host of the node>          # builds, loads relay-map:<commit> into containerd
ssh-keygen -t ed25519 -N '' -C relay-map-k8s -f id_ed25519
kubectl -n relay-map create secret generic relay-map-ssh --from-file=id_ed25519
kubectl apply -k <your overlay>
```

`build-image.sh` uses its own cargo target folder (`target/image`), so it never
replaces a hub running from `target/release`. It suits a single-node cluster
that runs images from its local containerd; push to a registry instead for more
nodes.
Set `CONTAINERD_SOCK` when the kubelet uses its own containerd socket (check
`containerRuntimeEndpoint` in /var/lib/kubelet/config.yaml).
