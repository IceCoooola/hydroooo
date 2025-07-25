use hydro_lang::*;
use hydro_std::bench_client::{bench_client, print_bench_results};

use super::two_pc::{Coordinator, Participant};
use crate::cluster::two_pc::two_pc;

pub struct Client;
pub struct Aggregator;

pub fn two_pc_bench<'a>(
    num_clients_per_node: usize,
    coordinator: &Process<'a, Coordinator>,
    participants: &Cluster<'a, Participant>,
    num_participants: usize,
    clients: &Cluster<'a, Client>,
    client_aggregator: &Process<'a, Aggregator>,
) {
    let bench_results = unsafe {
        bench_client(
            clients,
            |payloads| {
                // Add client's ID to payload so it can be sent back
                let self_addressed_payloads = payloads
                    .map(q!(move |payload| (CLUSTER_SELF_ID, payload)))
                    .send_bincode_anonymous(coordinator);
                // Send committed requests back to the original client
                two_pc(
                    coordinator,
                    participants,
                    num_participants,
                    self_addressed_payloads,
                )
                .send_bincode(clients)
            },
            num_clients_per_node,
        )
    };

    print_bench_results(bench_results, client_aggregator, clients);
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use dfir_lang::graph::WriteConfig;
    use hydro_deploy::Deployment;
    use hydro_lang::Location;
    use hydro_lang::deploy::{DeployCrateWrapper, HydroDeploy, TrybuildHost};
    use hydro_lang::ir::deep_clone;
    use hydro_lang::rewrites::persist_pullup::persist_pullup;
    #[cfg(stageleft_runtime)]
    use hydro_lang::{Cluster, Process};
    use hydro_optimize::partition_node_analysis::partitioning_analysis;
    use hydro_optimize::repair::{cycle_source_to_sink_input, inject_id, inject_location};

    #[cfg(stageleft_runtime)]
    use crate::cluster::{
        two_pc::{Coordinator, Participant},
        two_pc_bench::{Aggregator, Client},
    };

    const NUM_PARTICIPANTS: usize = 3;

    #[cfg(stageleft_runtime)]
    fn create_two_pc<'a>(
        coordinator: &Process<'a, Coordinator>,
        participants: &Cluster<'a, Participant>,
        clients: &Cluster<'a, Client>,
        client_aggregator: &Process<'a, Aggregator>,
    ) {
        super::two_pc_bench(
            100,
            coordinator,
            participants,
            NUM_PARTICIPANTS,
            clients,
            client_aggregator,
        );
    }

    #[test]
    fn two_pc_ir() {
        let builder = hydro_lang::FlowBuilder::new();
        let coordinator = builder.process();
        let participants = builder.cluster();
        let clients = builder.cluster();
        let client_aggregator = builder.process();

        create_two_pc(&coordinator, &participants, &clients, &client_aggregator);
        let built = builder.with_default_optimize::<HydroDeploy>();

        hydro_lang::ir::dbg_dedup_tee(|| {
            insta::assert_debug_snapshot!(built.ir());
        });

        let preview = built.preview_compile();
        insta::with_settings!({snapshot_suffix => "coordinator_mermaid"}, {
            insta::assert_snapshot!(
                preview.dfir_for(&coordinator).to_mermaid(&WriteConfig {
                    no_subgraphs: true,
                    no_pull_push: true,
                    no_handoffs: true,
                    op_text_no_imports: true,
                    ..WriteConfig::default()
                })
            );
        });

        let preview = built.preview_compile();
        insta::with_settings!({snapshot_suffix => "participants_mermaid"}, {
            insta::assert_snapshot!(
                preview.dfir_for(&participants).to_mermaid(&WriteConfig {
                    no_subgraphs: true,
                    no_pull_push: true,
                    no_handoffs: true,
                    op_text_no_imports: true,
                    ..WriteConfig::default()
                })
            );
        });
    }

    #[tokio::test]
    async fn two_pc_some_throughput() {
        let builder = hydro_lang::FlowBuilder::new();
        let coordinator = builder.process();
        let participants = builder.cluster();
        let clients = builder.cluster();
        let client_aggregator = builder.process();

        create_two_pc(&coordinator, &participants, &clients, &client_aggregator);
        let mut deployment = Deployment::new();

        let nodes = builder
            .with_process(&coordinator, TrybuildHost::new(deployment.Localhost()))
            .with_cluster(
                &participants,
                (0..NUM_PARTICIPANTS).map(|_| TrybuildHost::new(deployment.Localhost())),
            )
            .with_cluster(&clients, vec![TrybuildHost::new(deployment.Localhost())])
            .with_process(
                &client_aggregator,
                TrybuildHost::new(deployment.Localhost()),
            )
            .deploy(&mut deployment);

        deployment.deploy().await.unwrap();

        let client_node = &nodes.get_process(&client_aggregator);
        let client_out = client_node.stdout_filter("Throughput:").await;

        deployment.start().await.unwrap();

        use std::str::FromStr;

        use regex::Regex;

        let re = Regex::new(r"Throughput: ([^ ]+) - ([^ ]+) - ([^ ]+) requests/s").unwrap();
        let mut found = 0;
        let mut client_out = client_out;
        while let Some(line) = client_out.recv().await {
            if let Some(caps) = re.captures(&line) {
                if let Ok(lower) = f64::from_str(&caps[1]) {
                    if lower > 0.0 {
                        println!("Found throughput lower-bound: {}", lower);
                        found += 1;
                        if found == 2 {
                            break;
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn two_pc_partitionable() {
        let builder = hydro_lang::FlowBuilder::new();
        let coordinator = builder.process();
        let participants = builder.cluster();
        let clients = builder.cluster();
        let client_aggregator = builder.process();

        create_two_pc(&coordinator, &participants, &clients, &client_aggregator);

        let mut cycle_data = HashMap::new();
        let built = builder
            .optimize_with(persist_pullup)
            .optimize_with(inject_id)
            .optimize_with(|ir| {
                cycle_data = cycle_source_to_sink_input(ir);
                inject_location(ir, &cycle_data);
            })
            .into_deploy::<HydroDeploy>();
        let mut ir = deep_clone(built.ir());

        let coordinator_partitioning =
            partitioning_analysis(&mut ir, &coordinator.id(), &cycle_data);

        // 8 and 26 are the op_ids of the input nodes to the coordinator. They will change if the coordinator's logic changes
        // 1 is the partitioning index of those inputs. Specifically, given the client sends (sender_id, payload) to the coordinator, we can partition on the entire payload
        let expected_coordinator_partitioning = Some(vec![BTreeMap::from([
            (8, vec!["1".to_string()]),
            (26, vec!["1".to_string()]),
        ])]);

        assert_eq!(coordinator_partitioning, expected_coordinator_partitioning);

        let participant_partitioning =
            partitioning_analysis(&mut ir, &participants.id(), &cycle_data);

        // Participants can partition on ANYTHING, since they only execute maps
        let expected_participant_partitionings = Some(vec![]);

        assert_eq!(participant_partitioning, expected_participant_partitionings);
    }
}
