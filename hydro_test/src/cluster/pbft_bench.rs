use hydro_lang::*;
use hydro_std::quorum::collect_quorum;

use super::bench_client::{bench_client, Client};
use super::kv_replica::{kv_replica, KvPayload, Replica};
// use super::kv_replica::{kv_replica, KvPayload};
use super::paxos_with_client::PaxosLike;
use super::pbft::CorePbft;
// use super::pbft::Replica;
pub fn pbft_bench<'a>(
    flow: &FlowBuilder<'a>,
    num_clients_per_node: usize,
    median_latency_window_size: usize, /* How many latencies to keep in the window for calculating the median */
    checkpoint_frequency: usize,       // How many sequence numbers to commit before checkpointing
    f: usize, /* Maximum number of faulty nodes. A payload has been processed once f+1 replicas have processed it. */
    num_replicas: usize,
    create_pbft: impl FnOnce(Stream<usize, Cluster<'a, Replica>, Unbounded>) -> CorePbft<'a>,
    replicas: Cluster<'a, Replica>,
) -> Cluster<'a, Client> {
    let clients = flow.cluster::<Client>();
    // let replicas = flow.cluster::<Replica>();

    bench_client(
        &clients,
        |c_to_primary| {
            let payloads = c_to_primary.map(q!(move |(key, value)| KvPayload {
                key,
                // we use our ID as part of the value and use that so the replica only notifies us
                value: (CLUSTER_SELF_ID, value)
            }));

            let (replica_checkpoint_complete, replica_checkpoint) =
                replicas.forward_ref::<Stream<_, _, _>>();

            let pbft = create_pbft(replica_checkpoint);
        

            let sequenced_payloads = unsafe {
                // SAFETY: clients "own" certain keys, so interleaving elements from clients will not affect
                // the order of writes to the same key

                // TODO(shadaj): we should retry when a payload is dropped due to stale leader
                pbft.with_client(&clients, payloads) //return Paxos Out. 
                // put last phase in here.
            };

            // sequenced_payloads.clone().for_each(q!(|(a, b)|println!("{} --- {:?}", a, b)));

            // Replicas
            let (replica_checkpoint, processed_payloads) =
                kv_replica(&replicas, sequenced_payloads, checkpoint_frequency);

            replica_checkpoint_complete.complete(replica_checkpoint);
            
            // processed_payloads.clone().for_each(q!(|payload| println!("processed: {:?}", payload)));

            let c_received_payloads = processed_payloads
                .map(q!(|payload| (
                    payload.value.0,
                    ((payload.key, payload.value.1), Ok(()))
                )))
                .send_bincode_anonymous(&clients);

            // we only mark a transaction as committed when all replicas have applied it
            collect_quorum::<_, _, _, ()>(
                c_received_payloads.atomic(&clients.tick()),
                f + 1,
                num_replicas,
            )
            .0
            .end_atomic()

        },
        num_clients_per_node,
        median_latency_window_size,
    );

    clients
}


// #[cfg(test)]
// mod tests {
//     use hydro_lang::deploy::DeployRuntime;
//     use stageleft::RuntimeData;

//     use crate::cluster::paxos::{CorePaxos, PaxosConfig};

//     #[test]
//     fn paxos_ir() {
//         let builder = hydro_lang::FlowBuilder::new();
//         let proposers = builder.cluster();
//         let acceptors = builder.cluster();

//         let _ = super::pbft_bench(&builder, 1, 1, 1, 1, 2, |replica_checkpoint| CorePaxos {
//             proposers,
//             acceptors: acceptors.clone(),
//             replica_checkpoint: replica_checkpoint.broadcast_bincode(&acceptors),
//             paxos_config: PaxosConfig {
//                 f: 1,
//                 i_am_leader_send_timeout: 1,
//                 i_am_leader_check_timeout: 1,
//                 i_am_leader_check_timeout_delay_multiplier: 1,
//             },
//         });
//         let built = builder.with_default_optimize::<DeployRuntime>();

//         hydro_lang::ir::dbg_dedup_tee(|| {
//             insta::assert_debug_snapshot!(built.ir());
//         });

//         let _ = built.compile(&RuntimeData::new("FAKE"));
//     }
// }


// use hydro_lang::*;
// use hydro_std::quorum::collect_quorum;
// use super::pbft::Request;
// use super::bench_client::{bench_client, Client};
// use super::kv_replica::{kv_replica, KvPayload, Replica};
// use super::paxos_with_client::PaxosLike;


// pub fn pbft_bench<'a, Pbft: PaxosLike<'a>>(
//     flow: &FlowBuilder<'a>,
//     num_replicas: u32,
//     num_clients: u32,
//     f : u32,
//     median_latency_window_size: usize, /* How many latencies to keep in the window for calculating the median */
//     checkpoint_frequency: usize,       // How many sequence numbers to commit before checkpointing
//     create_pbft: impl FnOnce(Stream<usize, Cluster<'a, Replica>, Unbounded>) -> Pbft,

// ) -> (Cluster<'a, Client>, Cluster<'a, Replica>) {
//     let clients = flow.cluster::<Client>();

//     let replicas = flow.cluster::<Replica>();

//     bench_client(
//         &clients,
//         |c_to_primary| {
//             // put something for test.
//             let test_input = clients.source_iter(q!([0 as usize]));
//             let test_input_replica = replicas.source_iter(q!([0 as usize]));
//             let pbft = create_pbft(test_input_replica);
//             let test_output = unsafe{pbft.with_client(&clients, test_input)};
//             test_output.map(q!(|(a, b)| a))
//         },
//         num_clients as usize,
//         median_latency_window_size,
//     );

//     (clients, replicas)
// }
