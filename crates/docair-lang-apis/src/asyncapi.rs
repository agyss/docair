//! AsyncAPI 2.x and 3.x: the topics an application sends to and receives from, with the messages.

use crate::model::{Action, Api, Event, Format};
use crate::tree::{Node, Problem, ref_name, type_name};

/// Read an AsyncAPI document.
pub fn read(root: &Node) -> Result<Api, Problem> {
    let key = root
        .get("asyncapi")
        .ok_or_else(|| Problem::at(1, "no `asyncapi` version"))?;
    let version = key.text().unwrap_or_default();
    let major = version.split('.').next().unwrap_or("");
    if major != "2" && major != "3" {
        return Err(Problem::at(
            key.line,
            format!("AsyncAPI version `{version}` is not one docair reads (2.x, 3.x)"),
        ));
    }
    let mut api = Api::new(Format::AsyncApi, version.clone(), key.line);
    let info = root.get("info");
    api.title = info
        .and_then(|i| i.get("title"))
        .and_then(Node::words)
        .unwrap_or_else(|| "API".to_string());
    api.description = info
        .and_then(|i| i.get("description"))
        .and_then(Node::words);
    api.part_of = root.get("x-docair-part-of").and_then(Node::words);
    if let Some(servers) = root.get("servers") {
        for (_, _, server) in servers.entries() {
            if let Some(p) = server.get("protocol").and_then(Node::words)
                && !api.protocols.contains(&p)
            {
                api.protocols.push(p);
            }
        }
    }
    if major == "2" {
        read_2(root, &mut api);
    } else {
        read_3(root, &mut api);
    }
    Ok(api)
}

/// 2.x: channels are keyed by their address. `subscribe` lists what the application
/// sends (others subscribe to it), `publish` what it receives.
fn read_2(root: &Node, api: &mut Api) {
    let Some(channels) = root.get("channels") else {
        return;
    };
    for (address, _, channel) in channels.entries() {
        for (verb, verb_key, op) in channel.entries() {
            let action = match verb.as_str() {
                "subscribe" => Action::Send,
                "publish" => Action::Receive,
                _ => continue,
            };
            api.events.push(Event {
                action,
                address: address.clone(),
                message: op.get("message").and_then(|m| message_name(root, m)),
                id: op.get("operationId").and_then(Node::words),
                summary: op.get("summary").and_then(Node::words),
                description: op
                    .get("description")
                    .or_else(|| channel.get("description"))
                    .and_then(Node::words),
                line: verb_key.line,
            });
        }
    }
}

/// 3.x: operations name their `action` and point at a channel, whose `address` is the topic.
fn read_3(root: &Node, api: &mut Api) {
    let Some(operations) = root.get("operations") else {
        return;
    };
    for (id, key, op) in operations.entries() {
        let action = match op.get("action").and_then(Node::text).as_deref() {
            Some("send") => Action::Send,
            Some("receive") => Action::Receive,
            other => {
                api.skipped.push(Problem::at(
                    key.line,
                    format!(
                        "operation `{id}`: action `{}` is neither send nor receive",
                        other.unwrap_or("")
                    ),
                ));
                continue;
            }
        };
        let Some(reference) = op
            .get("channel")
            .and_then(|c| c.get("$ref"))
            .and_then(Node::text)
        else {
            api.skipped.push(Problem::at(
                key.line,
                format!("operation `{id}` names no channel"),
            ));
            continue;
        };
        let Some(channel) = root.resolve(&reference) else {
            api.skipped.push(Problem::at(
                key.line,
                format!("operation `{id}`: channel `{reference}` is not in this file"),
            ));
            continue;
        };
        let address = channel
            .get("address")
            .and_then(Node::text)
            .or_else(|| ref_name(&reference))
            .unwrap_or_default();
        let listed = op.get("messages").map(Node::items).unwrap_or(&[]);
        let all: Vec<&Node> = channel
            .get("messages")
            .map(|m| m.entries().map(|(_, _, v)| v).collect())
            .unwrap_or_default();
        let messages: Vec<&Node> = if listed.is_empty() {
            all
        } else {
            listed.iter().collect()
        };
        api.events.push(Event {
            action,
            address,
            message: names(root, &messages),
            id: Some(id.clone()),
            summary: op
                .get("summary")
                .or_else(|| channel.get("summary"))
                .and_then(Node::words),
            description: op
                .get("description")
                .or_else(|| channel.get("description"))
                .and_then(Node::words),
            line: key.line,
        });
    }
}

/// The names of several messages: `A | B`.
fn names(root: &Node, messages: &[&Node]) -> Option<String> {
    let names: Vec<String> = messages
        .iter()
        .filter_map(|m| message_name(root, m))
        .collect();
    (!names.is_empty()).then(|| names.join(" | "))
}

/// A message's `name`, else its `title`, else its payload's type, else the key it is filed under.
fn message_name(root: &Node, m: &Node) -> Option<String> {
    if let Some(list) = m.get("oneOf") {
        return names(root, &list.items().iter().collect::<Vec<_>>());
    }
    let reference = m.get("$ref").and_then(Node::text);
    let target = m.follow(root);
    target
        .get("name")
        .and_then(Node::words)
        .or_else(|| target.get("title").and_then(Node::words))
        .or_else(|| target.get("payload").and_then(type_name))
        .or_else(|| reference.as_deref().and_then(ref_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::parse;

    fn events(yaml: &str) -> Vec<(Action, String, Option<String>)> {
        read(&parse(yaml).unwrap())
            .unwrap()
            .events
            .into_iter()
            .map(|e| (e.action, e.address, e.message))
            .collect()
    }

    #[test]
    fn version_2_and_3_of_one_api_say_the_same() {
        let v2 = "asyncapi: 2.6.0
info: {title: Orders events, version: 1.0.0}
channels:
  order.created:
    subscribe:
      message: {$ref: '#/components/messages/OrderCreated'}
  order.cancelled:
    publish:
      message:
        oneOf:
          - {name: OrderCancelled}
          - {$ref: '#/components/messages/Refund'}
components:
  messages:
    OrderCreated: {name: OrderCreated, payload: {type: object}}
    Refund: {payload: {$ref: '#/components/schemas/RefundIssued'}}
";
        let v3 = "asyncapi: 3.0.0
info: {title: Orders events, version: 1.0.0}
channels:
  created:
    address: order.created
    messages:
      OrderCreated: {$ref: '#/components/messages/OrderCreated'}
  cancelled:
    address: order.cancelled
    messages:
      OrderCancelled: {name: OrderCancelled}
      Refund: {$ref: '#/components/messages/Refund'}
operations:
  sendCreated:
    action: send
    channel: {$ref: '#/channels/created'}
  receiveCancelled:
    action: receive
    channel: {$ref: '#/channels/cancelled'}
components:
  messages:
    OrderCreated: {name: OrderCreated, payload: {type: object}}
    Refund: {payload: {$ref: '#/components/schemas/RefundIssued'}}
";
        let want = vec![
            (
                Action::Send,
                "order.created".to_string(),
                Some("OrderCreated".to_string()),
            ),
            (
                Action::Receive,
                "order.cancelled".to_string(),
                Some("OrderCancelled | RefundIssued".to_string()),
            ),
        ];
        assert_eq!(events(v2), want);
        assert_eq!(events(v3), want);
    }

    #[test]
    fn a_channel_without_address_is_its_key_and_bad_operations_are_said() {
        let api = read(
            &parse(
                "asyncapi: 3.0.0
info: {title: X}
servers: {prod: {host: k:9092, protocol: kafka}, dev: {host: d, protocol: kafka}}
channels:
  jobs: {address: null}
operations:
  take: {action: receive, channel: {$ref: '#/channels/jobs'}}
  odd: {action: watch, channel: {$ref: '#/channels/jobs'}}
",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(api.events[0].address, "jobs");
        assert_eq!(api.protocols, vec!["kafka".to_string()]);
        assert_eq!(api.skipped[0].line, Some(8));
    }
}
