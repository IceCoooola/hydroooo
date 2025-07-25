use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use hydro_lang::*;
use hydro_std::quorum::{collect_quorum, collect_quorum_with_response};
use hydro_std::request_response::join_responses;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use super::paxos::PaxosPayload;
use super::paxos_with_client::PaxosLike;
use std::collections::hash_map::DefaultHasher;
use super::pbft as helper;
use super::kv_replica::Replica;

// pub struct Replica {}

//unwrap_or. 
fn calculate_hash<T: Hash>(t: &T) -> u64 {
    // let mut s = DefaultHasher::new();
    // t.hash(&mut s);
    // s.finish()
    0
} 

#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Hash, Ord, PartialOrd)]
pub struct Request {
    pub request: u32, // currently set the request to unsigned int
    pub t: u32,       // a unique request timestamp id for the client.
    pub client_id: u32,
    pub client_signature: u64, // currently set it to 0.
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Hash)]
pub struct Reply {
    pub view_no: u32, // current view number
    pub t: u32,       // a unique request id for the client.
    pub client_id: u32,
    pub replica_id: u32, // replica' id
    pub result: bool,                    // currently set the result of this request to bool
    // pub client_signature: u32,         // currently set it to 0.
}

// PRE-PREPARE messages are used as proof m was assigned seq num n in view v during view changes
#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Hash)]
pub struct PrePrepare<P> {
    pub view_no: u32,             // current view number
    pub n: u32,                   // sequence number
    pub digest: u64,              // currently set the digest to a u32
    pub primary_signature: u64, // currently set it to 1.
    pub message: P,             // the message
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Hash)]
pub struct Prepare {
    pub view_no: u32,             // current view number
    pub n: u32,                   // sequence number
    pub digest: u64,              // currently set the digest to a u32
    pub i: u32,                  // replica number i
    pub replica_signature: u64,  // currently set it to 1.
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Hash)]
pub struct Prepared<P> {
    pub message: P, // the message
    pub view_no: u32, // current view number
    pub n: u32,       // sequence number
    pub i: u32,     // replica number i
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Hash)]
pub struct Commit {
    pub view_no: u32,             // current view number
    pub n: u32,                   // sequence number
    pub digest: u64,              // currently set the digest to a u32
    pub i: u32,                 // replica number i
    pub replica_signature: u64, 
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Hash)]
pub struct Committed<P> {
    pub message: P,             // current message
    pub view_no: u32,                   // current view number
    pub n: u32,              // sequence number
    pub i: u32,                 // replica number i
}

// <VIEW-CHANGE, v + 1, n, C, P, i>sig(i): replica i wants to move into view v + 1, has a stable checkpoint s at seq num n proven correct by C, and has a set P of messages proving requests after the last stable checkpoint’s seq num n satisfied prepared
#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug, Hash)]
pub struct ViewChange {
    pub view_no: u32,       // the new view number that replica want to move to
    pub n: u32,           // a stable checkpoint s at seq num n proven correct by C
    pub check_point: u32, // stable check point C
    //TODO: the P set contains the set Pm of PRE-PREPARE + 2f PREPARE messages for any non-checkpointed message m that satisfied prepared
    pub P: u32, // TODO: set P of messages proving requests after the last stable checkpoint’s seq num n satisfied prepared
    pub i: u32,          // replica number i
    pub replica_signature: u64, 
    pub replica_id: ClusterId<Replica>,
}


impl Ord for ViewChange {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.n
            .cmp(&other.n)
            .then_with(|| self.replica_id.raw_id.cmp(&other.replica_id.raw_id))
    }
}

impl PartialOrd for ViewChange {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}


#[derive(Clone, Copy)]
pub struct PbftConfig {
    /// Maximum number of faulty nodes
    pub f: usize,
    /// How often to send "I am leader" heartbeats
    pub i_am_leader_send_timeout: u64,
    /// How often to check if the leader has expired
    pub i_am_leader_check_timeout: u64,
    /// Initial delay, multiplied by proposer pid, to stagger proposers checking for timeouts
    pub i_am_leader_check_timeout_delay_multiplier: usize,
}

pub struct CorePbft<'a> {
    pub replicas: Cluster<'a, Replica>,
    pub pbft_config: PbftConfig,
}


impl<'a> PaxosLike<'a> for CorePbft<'a> {
    type PaxosIn = Replica;
    type PaxosOut = Replica;
    type PaxosLog = Replica;
    type Ballot = ViewChange; 

    fn payload_recipients(&self) -> &Cluster<'a, Self::PaxosIn> {
        &self.replicas
    }

    fn log_stores(&self) -> &Cluster<'a, Self::PaxosLog> {
        &self.replicas
    }

    fn get_recipient_from_ballot<L: Location<'a>>(
        view: Optional<Self::Ballot, L, Unbounded>,
    ) -> Optional<ClusterId<Replica>, L, Unbounded> {
        view.map(q!(|view| view.replica_id))
    }

    unsafe fn build<P: PaxosPayload>(
        self,
        payload_generator: impl FnOnce(
            Stream<Self::Ballot, Cluster<'a, Self::PaxosIn>, Unbounded>,
        ) -> Stream<P, Cluster<'a, Self::PaxosIn>, Unbounded>,
        checkpoints: Optional<usize, Cluster<'a, Self::PaxosLog>, Unbounded>,
    ) -> Stream<(usize, Option<P>), Cluster<'a, Self::PaxosOut>, Unbounded, NoOrder> {
        unsafe {
            pbft_core(
                &self.replicas,
                checkpoints,
                payload_generator,
                self.pbft_config,
            )
        }
    }
}


// pub unsafe fn pbft_core<'a, P: PaxosPayload>(
//     replicas: &Cluster<'a, Replica>,
//     view_checkpoint: Stream<(ClusterId<Replica>, usize), Cluster<'a, Replica>, Unbounded, NoOrder>,
//     c_to_primary: impl FnOnce(
//         Stream<ViewChange, Cluster<'a, Replica>, Unbounded>,
//     ) -> Stream<P, Cluster<'a, Replica>, Unbounded>,
//     pbft_config: PbftConfig,
// ) -> Stream<(usize, Option<P>), Cluster<'a, Replica>, Unbounded, NoOrder> {
//     let f = pbft_config.f;
//         /// How often to send "I am leader" heartbeats
//     let i_am_leader_send_timeout = pbft_config.i_am_leader_send_timeout;
//         /// How often to check if the leader has expired
//     let i_am_leader_check_timeout = pbft_config.i_am_leader_check_timeout;
//         /// Initial delay, multiplied by proposer pid, to stagger proposers checking for timeouts
//     let i_am_leader_check_timeout_delay_multiplier = pbft_config.i_am_leader_check_timeout_delay_multiplier;
    
//     replicas
//         .source_iter(q!(["Replicas say hello"]))
//         .for_each(q!(|s| println!("{}", s)));
    
//     let replicas_tick = replicas.tick();
    
//     let (r_pre_prepare_log_complete_cycle, r_pre_prepare_log) = replicas.forward_ref::<Stream<(u32, PrePrepare<P>), _, _, _>>();
    

//     let max_checkpoint_n = view_checkpoint
//     .map(q!(|(id, n)| n))
//     .tick_batch(&replicas_tick)
//     .max()
//     .into_singleton();


//     let (r_view_number_complete_cycle, r_view_number) = replicas_tick.cycle_with_initial(replicas_tick.singleton(q!(ViewChange {
//         view_no: 0,      
//         n: 0,
//         check_point: 0,
//         P: 0,
//         i: 1,
//         replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
//         replica_id: CLUSTER_SELF_ID,
//     })));


//     // TODO: trigger view change here if view_number no found. 
//     // r_view_number.clone().all_ticks().for_each(q!(|v| println!("view {:?}", v)));
    
//     // tell the client that the current view is this, and get the actual payload. 
//     let c_to_primary = unsafe {
//             c_to_primary(
//             replicas_tick.optional_first_tick(q!(ViewChange {
//                 view_no: 0,      
//                 n: 0,
//                 check_point: 0,
//                 P: 0,
//                 i: 1,
//                 replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
//                 replica_id: CLUSTER_SELF_ID,
//             })).filter(q!(move |_view| CLUSTER_SELF_ID.raw_id == 0)).into_stream().all_ticks()
//         ).tick_batch(&replicas_tick)
//     };
//     // }.inspect(q!(|payload| println!("Replica received payload: {:?}", payload)));
    
//     r_view_number_complete_cycle.complete_next_tick(r_view_number.clone());

//     // if view == current id number ,then assume it is the primary. 
//     let p_receive_request = c_to_primary
//     .cross_singleton(r_view_number.clone())
//     .filter_map(q!(move |(request, view_change)| {
//     // if the current node is the primary, then Some(request), otherwise None.
//     if CLUSTER_SELF_ID.raw_id == view_change.view_no as u32 % (3 * f as u32 + 1) {
//         Some(request)
//     } else {
//         None
//     }}));

//     // p_receive_request.clone().all_ticks().for_each(q!(|request| println!("primary received request: {:?}", request)));

//     let p_next_sequence_num = r_pre_prepare_log.clone()
//     .map(q!(|(n, _pre_prepare)| n))
//     .max()
//     .unwrap_or(replicas.singleton(q!(0)));

//     // (primary) orders request, multicast PRE-PREPARE → all backups
//     let p_pre_prepare = p_receive_request
//         .clone()
//         .cross_singleton(r_view_number.clone())
//         .cross_singleton(p_next_sequence_num.clone().latest_tick(&replicas_tick))
//         .enumerate()
//         .all_ticks()
//         .map(q!(move |(i, ((request, view_change), n))|

//             PrePrepare {
//             view_no: view_change.view_no,     // current view number
//             n: n + i as u32,      // sequence number
//             digest: helper::calculate_hash(&request), // currently set the digest to request's client timestamp
//             primary_signature: helper::calculate_hash(&CLUSTER_SELF_ID),
//             message: request, // the message
//         }
//     ));

//     // p_pre_prepare
//     //     .clone()
//     //     .for_each(q!(|pre_prepare| println!(
//     //         "primary log pre-prepare: {:?}",
//     //         pre_prepare
//     //     )));

//     let r_pre_prepare = p_pre_prepare.broadcast_bincode_anonymous(&replicas);
//     // TODO: verify should inclues:sig(p) is valid, d is digest of m, the replica is in view v
//     // TODO: the backup hasn’t accepted another PRE-PREPARE message for view v and seq number n with another digest
//     // TODO: seq num n is between a low watermark h and high watermark H
//     let r_verified_pre_prepare = r_pre_prepare
//         .clone()
//         .tick_batch(&replicas_tick)
//         .cross_singleton(r_view_number.clone())
//         .all_ticks()
//         .filter_map(q!(move |(pre_prepare, view_change)| 
//         if  pre_prepare.view_no == view_change.view_no
//             && helper::calculate_hash(&pre_prepare.message) == pre_prepare.digest 
//         {
//             Some((pre_prepare.n, pre_prepare))
//         } else {
//             None
//         }));

//     // r_verified_pre_prepare.clone().for_each(q!(|(n, preprepare)| println!("before filter: n-->{} --> {:?}", n, preprepare)));
//     // checkpoint_quorom_reached.for_each(q!(|ck| println!("view check point quorom reached: {:?}", ck)));
//     let r_verified_pre_prepare_after_checkpoint = r_verified_pre_prepare
//     .clone()
//     .tick_batch(&replicas_tick)
//     .cross_singleton(max_checkpoint_n)
//     .filter_map(q!(|((n, pre_prepare), max_checkpoint_n)| {
//         if let Some(ckpt) = max_checkpoint_n  { 
//         if n >= ckpt as u32 {
//             Some((n, pre_prepare))
//         } else {
//             None
//         }
//     } else {
//         None
//     }
//     }))
//     .all_ticks();
//     // r_verified_pre_prepare_after_checkpoint.clone().for_each(q!(|(n, preprepare)| println!("after filter: n-->{} --> {:?}", n, preprepare)));

//     r_pre_prepare_log_complete_cycle.complete(r_verified_pre_prepare_after_checkpoint.clone());
//     // r_pre_prepare_log
//     //     .clone()
//     //     .for_each(q!(|pre_prepare| println!(
//     //         "replicas log pre-prepare: {:?}",
//     //         pre_prepare
//     //     )));

    
//     let r_prepare_ready_to_send =
//         r_verified_pre_prepare.map(q!(move |(n, pre_prepare)| 
//         (
//             n, 
//             Prepare {
//             view_no: pre_prepare.view_no,  // current view number
//             n: pre_prepare.n,              // sequence number
//             digest: pre_prepare.digest,    // currently set the digest to a u32
//             i: CLUSTER_SELF_ID.raw_id,                 // replica number i
//             replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
//         }
//         )
//     ));

//     // replica i broadcasts PREPARE messages to all other replicas, including primary. TODO: fix later.
//     let r_receive_prepare = r_prepare_ready_to_send.broadcast_bincode_anonymous(&replicas);

//     // TODO: validate prepare message. sig(i) is correct, replica i has the same pre_prepare message, the replica is in view v, h < n < H
//     let r_verified_prepare = r_receive_prepare
//         .join(r_pre_prepare_log.clone())
//         .tick_batch(&replicas_tick)
//         .cross_singleton(r_view_number.clone())
//         .all_ticks()
//         .filter_map(q!(
//             |((n, (prepare, pre_prepare)), view_change)| {
//             // println!("Prepare view {}, self view {}, prepare digest {}, prepare digest {}", prepare.view_no, view_change.view_no, pre_prepare.digest, prepare.digest);
//             if (prepare.view_no == view_change.view_no
//                 && pre_prepare.digest == prepare.digest)
//             {
//                 Some((n, prepare))
//             } else {
//                 None
//             }
//     }));


//     let r_prepare_log = r_verified_prepare.clone();
//     // r_prepare_log.for_each(q!(|prepare| println!("r_verified_prepare: {:?}", prepare)));

//     // count the prepare message.  2f PREPARE messages from different backups matching the PRE-PREPARE
//     let (r_verified_prepare_quorum_reached, _) = collect_quorum(
//         r_verified_prepare
//             .clone()
//             .map(q!(move |(n, prepare)| (
//                 n,
//                 Ok::<(), ()>(())
//                 // if prepare.i != CLUSTER_SELF_ID.raw_id { // if it is not from myself. 
//                 //     Ok(())
//                 // } else {
//                 //     Err(())
//                 // }
//             )))
//             .atomic(&replicas_tick),
//         2 * f + 1, 
//         3 * f + 1,
//     );
//     // r_verified_prepare_quorum_reached
//     //     .clone()
//     //     .for_each(q!(|prepare| println!("prepare quorum reached!!{:?}", prepare)));

//     let r_ready_prepare = r_verified_prepare_quorum_reached
//         .end_atomic()
//         .map(q!(|n| (n, n)))
//         .join(r_verified_prepare)
//         .map(q!(move |(n, (_n, prepare))| (n, prepare)));
//         // .map(q!(move |(n, (_n, prepare))| if (prepare.digest == digest && prepare.i == CLUSTER_SELF_ID.raw_id) {
//         //     Some((prepare.n, prepare))
//         // } else {
//         //     None
//         // }));

//     let r_prepared = r_ready_prepare
//         .atomic(&replicas_tick)
//         .join(r_pre_prepare_log.clone().atomic(&replicas_tick))
//         .filter_map(q!(|(_n, (prepare, pre_prepare))| if (prepare.digest == pre_prepare.digest) {
//             Some(( 
//                 pre_prepare.n,
//                 Prepared {
//                 message: pre_prepare.message, // the message
//                 view_no: pre_prepare.view_no, // current view number
//                 n: pre_prepare.n,             // sequence number
//                 i: prepare.i,                 // replica number i
//             })
//             )
//         } else {
//             None
//         }
//         ));

//     let r_commit_ready_to_send = r_prepared.clone().map(q!(move |(n, prepared)| 
//         (
//         n, 
//         Commit {
//         view_no: prepared.view_no,                          // current view number
//         n: prepared.n,                                      // sequence number
//         digest: helper::calculate_hash(&prepared.message),  // currently set the digest to a u32
//         i: prepared.i,                                      // replica number i
//         replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
//         }
//         )
//     ));

//     // broadcast a COMMIT message
//     let r_receive_commit = r_commit_ready_to_send.broadcast_bincode_anonymous(&replicas);

//     // r_receive_commit.clone().for_each(q!(|(id, commit)| println!(
//     //     "replicas receive commit from {} and the message: {:?}",
//     //     id, commit
//     // )));
//     // validate commit message. sig(i) is correct, replica i has the same pre_prepare message
//     // TODO: the replica is in view v, h < n < H
//     let r_verified_commit = r_receive_commit
//         .join(r_prepared.end_atomic())
//         .filter_map(q!(
//             |(n, (commit, prepared))| if (commit.view_no == prepared.view_no
//                 && commit.digest == helper::calculate_hash(&prepared.message))
//             {
//                 Some((n, commit))
//             } else {
//                 None
//             }
//         ));


//     // count the commit message.  2f COMMIT messages from different backups matching the PREPARED
//     let (r_verified_commit_digest_quorum_reached, _) = collect_quorum(
//         r_verified_commit
//             .map(q!(move |(n, commit)| (
//                 (n, commit.digest),
//                 if commit.replica_signature != helper::calculate_hash(&CLUSTER_SELF_ID) {
//                     Ok(())
//                 } else {
//                     Err(())
//                 }
//             )))
//             .atomic(&replicas_tick),
//             2 * f, 
//             3 * f + 1,
//     );

//     // r_verified_commit_digest_quorum_reached.clone().for_each(q!(move |commit| println!(
//     //     "These operation will be executed on replica {}: {:?}", CLUSTER_SELF_ID.raw_id, 
//     //     commit
//     // )));

//     let r_committed = r_verified_commit_digest_quorum_reached.end_atomic()
//     .join(r_pre_prepare_log.clone()).filter_map(
//         q!(move |(n, (digest, pre_prepare))| if pre_prepare.digest == digest {
//             Some((
//                 n, 
//                 Committed {
//                 message: pre_prepare.message,
//                 view_no: pre_prepare.view_no,
//                 n: pre_prepare.n,
//                 i: CLUSTER_SELF_ID.raw_id
//             }))
//         } else {
//             None
//         })
//     );

//     let r_committed_log = r_committed.clone();

//     let r_ready_send_reply = r_committed.map(q!(move |(n, committed)|
//         (n as usize, Some(committed.message))
//     ));

//     r_ready_send_reply
// }



















pub unsafe fn pbft_core<'a, P: PaxosPayload>(
    replicas: &Cluster<'a, Replica>,
    view_checkpoint: Optional<usize, Cluster<'a, Replica>, Unbounded>,
    c_to_primary: impl FnOnce(
        Stream<ViewChange, Cluster<'a, Replica>, Unbounded>,
    ) -> Stream<P, Cluster<'a, Replica>, Unbounded>,
    pbft_config: PbftConfig,
) -> Stream<(usize, Option<P>), Cluster<'a, Replica>, Unbounded, NoOrder> {
    let f = pbft_config.f;
    //     /// How often to send "I am leader" heartbeats
    // let i_am_leader_send_timeout = pbft_config.i_am_leader_send_timeout;
    //     /// How often to check if the leader has expired
    // let i_am_leader_check_timeout = pbft_config.i_am_leader_check_timeout;
    //     /// Initial delay, multiplied by proposer pid, to stagger proposers checking for timeouts
    // let i_am_leader_check_timeout_delay_multiplier = pbft_config.i_am_leader_check_timeout_delay_multiplier;
    
    replicas
        .source_iter(q!(["Replicas say hello"]))
        .for_each(q!(|s| println!("{}", s)));

    let replicas_tick = replicas.tick();

    let (seq_complete_cycle, seq) = replicas_tick.cycle_with_initial(replicas_tick.singleton(q!(0)));

    // let (r_pre_prepare_log_complete_cycle, r_pre_prepare_log) = replicas.forward_ref::<Stream<(u32, PrePrepare<P>), _, _, _>>();
    
    // let (r_pre_prepare_log_complete_cycle, r_pre_prepare_log) = replicas.forward_ref::<Stream<(usize, Option<P>), _, _, _>>();

    // let max_checkpoint_n = view_checkpoint
    // .map(q!(|(id, n)| n))
    // .tick_batch(&replicas_tick)
    // .max()
    // .into_singleton();

    let (r_view_number_complete_cycle, r_view_number) = replicas_tick.cycle_with_initial(replicas_tick.singleton(q!(ViewChange {
        view_no: 0,      
        n: 0,
        check_point: 0,
        P: 0,
        i: 1,
        replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
        replica_id: CLUSTER_SELF_ID,
    })));


    // TODO: trigger view change here if view_number no found. 
    // r_view_number.clone().all_ticks().for_each(q!(|v| println!("view {:?}", v)));
    
    // tell the client that the current view is this, and get the actual payload. 
    let c_to_primary = unsafe {
            c_to_primary(
            replicas_tick.optional_first_tick(q!(ViewChange {
                view_no: 0,      
                n: 0,
                check_point: 0,
                P: 0,
                i: 1,
                replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
                replica_id: CLUSTER_SELF_ID,
            })).filter(q!(move |_view| CLUSTER_SELF_ID.raw_id == 0)).into_stream().all_ticks()
        ).tick_batch(&replicas_tick)
    };

    // if view == current id number ,then assume it is the primary. 
    let p_receive_request = c_to_primary
    .cross_singleton(r_view_number.clone())
    .filter_map(q!(move |(request, view_change)| {
    // if the current node is the primary, then Some(request), otherwise None.
    if CLUSTER_SELF_ID.raw_id == view_change.view_no as u32 % (3 * f as u32 + 1) {
        Some(request)
    } else {
        None
    }}));

    // p_receive_request.clone().all_ticks().for_each(q!(|n| println!("received: {:?}", n)));

    let p_pre_prepare = p_receive_request
        .clone()
        .cross_singleton(r_view_number.clone())
        .cross_singleton(seq.clone())
        .enumerate()
        // .all_ticks()
        .map(q!(move |(i, ((request, view_change), n))|

            PrePrepare {
            view_no: view_change.view_no,     // current view number
            n: (n + i) as u32,      // sequence number
            digest: helper::calculate_hash(&request), // currently set the digest to request's client timestamp
            primary_signature: helper::calculate_hash(&CLUSTER_SELF_ID),
            message: request, // the message
        }
    ));
    let new_seq = p_receive_request.clone()
    .count()
    .zip(seq.clone())
    .map(q!(|(num_inputs, curr_seq)| num_inputs + curr_seq));

    seq_complete_cycle.complete_next_tick(new_seq);

    r_view_number_complete_cycle.complete_next_tick(r_view_number.clone());

    // // p_receive_request.clone().all_ticks().for_each(q!(|request| println!("primary received request: {:?}", request)));
    // let out = c_to_primary.clone().enumerate()
    // .cross_singleton(seq.clone()).map(q!(|((i, payload), seq)| {
    //     (seq + i, Some(payload))
    // }))
    // .all_ticks()
    // .broadcast_bincode_anonymous(replicas);

    // let p_next_sequence_num = r_pre_prepare_log.clone()
    // .map(q!(|(n, _pre_prepare)| n))
    // .max()
    // .unwrap_or(replicas.singleton(q!(0)));

    // (primary) orders request, multicast PRE-PREPARE → all backups

    // p_pre_prepare
    //     .clone()
    //     .for_each(q!(|pre_prepare| println!(
    //         "primary log pre-prepare: {:?}",
    //         pre_prepare
    //     )));
    // unsafe {    
    //     p_pre_prepare.clone().count().sample_eager().for_each(q!(|n| println!("send back: {:?}", n)));  
    // }

    let p_pre_prepare_ready_send = p_pre_prepare.all_ticks();
    //     unsafe {    
    //     p_pre_prepare_ready_send.clone().count().sample_eager().for_each(q!(|n| println!("send back: {:?}", n)));  
    // }
    let r_pre_prepare = p_pre_prepare_ready_send.broadcast_bincode_anonymous(&replicas);
    // unsafe {
    // r_pre_prepare.clone().tick_batch(&replicas_tick).count().all_ticks().for_each(q!(|n| println!("replicas received {}", n)));
    // }
    let out = r_pre_prepare.map(q!(|preprepare| (preprepare.n as usize, Some(preprepare.message))));
    // r_pre_prepare_log_complete_cycle.complete(out.clone());
    out

    // let out = c_to_primary.clone().enumerate()
    // .cross_singleton(seq.clone()).map(q!(|((i, payload), seq)| {
    //     (seq + i, Some(payload))
    // }))
    // .all_ticks()
    // .broadcast_bincode_anonymous(replicas);
    // let new_seq = c_to_primary.count().zip(seq).map(q!(|(num_inputs, curr_seq)| num_inputs + curr_seq));
    // seq_complete_cycle.complete_next_tick(new_seq);

    // out.into()

    // // TODO: verify should inclues:sig(p) is valid, d is digest of m, the replica is in view v
    // // TODO: the backup hasn’t accepted another PRE-PREPARE message for view v and seq number n with another digest
    // // TODO: seq num n is between a low watermark h and high watermark H
    // let r_verified_pre_prepare = r_pre_prepare
    //     .clone()
    //     .tick_batch(&replicas_tick)
    //     .cross_singleton(r_view_number.clone())
    //     .all_ticks()
    //     .filter_map(q!(move |(pre_prepare, view_change)| 
    //     if  pre_prepare.view_no == view_change.view_no
    //         && helper::calculate_hash(&pre_prepare.message) == pre_prepare.digest 
    //     {
    //         Some((pre_prepare.n, pre_prepare))
    //     } else {
    //         None
    //     }));

    // // r_verified_pre_prepare.clone().for_each(q!(|(n, preprepare)| println!("before filter: n-->{} --> {:?}", n, preprepare)));
    // // checkpoint_quorom_reached.for_each(q!(|ck| println!("view check point quorom reached: {:?}", ck)));
    // let r_verified_pre_prepare_after_checkpoint = r_verified_pre_prepare
    // .clone()
    // .tick_batch(&replicas_tick)
    // .cross_singleton(max_checkpoint_n)
    // .filter_map(q!(|((n, pre_prepare), max_checkpoint_n)| {
    //     if let Some(ckpt) = max_checkpoint_n  { 
    //     if n >= ckpt as u32 {
    //         Some((n, pre_prepare))
    //     } else {
    //         None
    //     }
    // } else {
    //     None
    // }
    // }))
    // .all_ticks();
    // // r_verified_pre_prepare_after_checkpoint.clone().for_each(q!(|(n, preprepare)| println!("after filter: n-->{} --> {:?}", n, preprepare)));

    // r_pre_prepare_log_complete_cycle.complete(r_verified_pre_prepare_after_checkpoint.clone());

    // let r_ready_send_reply = r_verified_pre_prepare_after_checkpoint
    // .map(q!(|(n, pre_prepare)| (n as usize, Some(pre_prepare.message))));

    // r_ready_send_reply
}






// pub unsafe fn pbft_core<'a, P: PaxosPayload> (
//     replicas: &Cluster<'a, Replica>,
//     view_checkpoint: Stream<(ClusterId<Replica>, usize), Cluster<'a, Replica>, Unbounded, NoOrder>,
//     c_to_primary: impl FnOnce(
//         Stream<ViewChange, Cluster<'a, Replica>, Unbounded>,
//     ) -> Stream<P, Cluster<'a, Replica>, Unbounded>,
//     pbft_config: PbftConfig,
// ) -> Stream<(usize, Option<P>), Cluster<'a, Replica>, Unbounded, NoOrder> {
//     let f = pbft_config.f;
//         /// How often to send "I am leader" heartbeats
//     let i_am_leader_send_timeout = pbft_config.i_am_leader_send_timeout;
//         /// How often to check if the leader has expired
//     let i_am_leader_check_timeout = pbft_config.i_am_leader_check_timeout;
//         /// Initial delay, multiplied by proposer pid, to stagger proposers checking for timeouts
//     let i_am_leader_check_timeout_delay_multiplier = pbft_config.i_am_leader_check_timeout_delay_multiplier;
    
//     replicas
//         .source_iter(q!(["Replicas say hello"]))
//         .for_each(q!(|s| println!("{}", s)));
    
//     let replicas_tick = replicas.tick();
    
//     let (r_pre_prepare_log_complete_cycle, r_pre_prepare_log) = replicas.forward_ref::<Stream<(u32, PrePrepare<P>), _, _, _>>();
    

//     let max_checkpoint_n = view_checkpoint
//     .map(q!(|(id, n)| n))
//     .tick_batch(&replicas_tick)
//     .max()
//     .into_singleton();


//     let (r_view_number_complete_cycle, r_view_number) = replicas_tick.cycle_with_initial(replicas_tick.singleton(q!(ViewChange {
//         view_no: 0,      
//         n: 0,
//         check_point: 0,
//         P: 0,
//         i: 1,
//         replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
//         replica_id: CLUSTER_SELF_ID,
//     })));


//     // TODO: trigger view change here if view_number no found. 
//     // r_view_number.clone().all_ticks().for_each(q!(|v| println!("view {:?}", v)));
    
//     // tell the client that the current view is this, and get the actual payload. 
//     let c_to_primary = unsafe {
//             c_to_primary(
//             replicas_tick.optional_first_tick(q!(ViewChange {
//                 view_no: 0,      
//                 n: 0,
//                 check_point: 0,
//                 P: 0,
//                 i: 1,
//                 replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
//                 replica_id: CLUSTER_SELF_ID,
//             })).filter(q!(move |_view| CLUSTER_SELF_ID.raw_id == 0)).into_stream().all_ticks()
//         ).tick_batch(&replicas_tick)
//     };
//     // }.inspect(q!(|payload| println!("Replica received payload: {:?}", payload)));
    
//     r_view_number_complete_cycle.complete_next_tick(r_view_number.clone());

//     // if view == current id number ,then assume it is the primary. 
//     let p_receive_request = c_to_primary
//     .cross_singleton(r_view_number.clone())
//     .filter_map(q!(move |(request, view_change)| {
//     // if the current node is the primary, then Some(request), otherwise None.
//     if CLUSTER_SELF_ID.raw_id == view_change.view_no as u32 % (3 * f as u32 + 1) {
//         Some(request)
//     } else {
//         None
//     }}));

//     // p_receive_request.clone().all_ticks().for_each(q!(|request| println!("primary received request: {:?}", request)));

//     let p_next_sequence_num = r_pre_prepare_log.clone()
//     .map(q!(|(n, _pre_prepare)| n))
//     .max()
//     .unwrap_or(replicas.singleton(q!(0)));

//     // (primary) orders request, multicast PRE-PREPARE → all backups
//     let p_pre_prepare = p_receive_request
//         .clone()
//         .cross_singleton(r_view_number.clone())
//         .cross_singleton(p_next_sequence_num.clone().latest_tick(&replicas_tick))
//         .enumerate()
//         .all_ticks()
//         .map(q!(move |(i, ((request, view_change), n))|

//             PrePrepare {
//             view_no: view_change.view_no,     // current view number
//             n: n + i as u32,      // sequence number
//             digest: helper::calculate_hash(&request), // currently set the digest to request's client timestamp
//             primary_signature: helper::calculate_hash(&CLUSTER_SELF_ID),
//             message: request, // the message
//         }
//     ));


//     let r_pre_prepare = p_pre_prepare.broadcast_bincode_anonymous(&replicas);

//     // r_pre_prepare.clone().for_each(q!(|pre_prepare| println!("receice pre-prepare: {:?}", pre_prepare)));

//     let r_verified_pre_prepare = r_pre_prepare
//     .map(q!(move |pre_prepare| (pre_prepare.n, pre_prepare)));

//     let r_verified_pre_prepare_after_checkpoint = r_verified_pre_prepare
//     .clone()
//     .tick_batch(&replicas_tick)
//     .cross_singleton(max_checkpoint_n)
//     .filter_map(q!(|((n, pre_prepare), max_checkpoint_n)| {
//         if let Some(ckpt) = max_checkpoint_n  { 
//         if n >= ckpt as u32 {
//             Some((n, pre_prepare))
//         } else {
//             None
//         }
//     } else {
//         None
//     }
//     }))
//     .all_ticks();
//     // r_verified_pre_prepare_after_checkpoint.clone().for_each(q!(|(n, preprepare)| println!("after filter: n-->{} --> {:?}", n, preprepare)));

//     r_pre_prepare_log_complete_cycle.complete(r_verified_pre_prepare_after_checkpoint.clone());

//     // r_pre_prepare_log
//     //     .clone()
//     //     .for_each(q!(|pre_prepare| println!(
//     //         "replicas log pre-prepare: {:?}",
//     //         pre_prepare
//     //     )));

    
//     let r_prepare_ready_to_send =
//         r_verified_pre_prepare.map(q!(move |(n, pre_prepare)| 
//         (
//             n, 
//             Prepare {
//             view_no: pre_prepare.view_no,  // current view number
//             n: pre_prepare.n,              // sequence number
//             digest: pre_prepare.digest,    // currently set the digest to a u32
//             i: CLUSTER_SELF_ID.raw_id,                 // replica number i
//             replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
//         }
//         )
//     ));

//     // replica i broadcasts PREPARE messages to all other replicas, including primary. TODO: fix later.
//     let r_receive_prepare = r_prepare_ready_to_send.broadcast_bincode_anonymous(&replicas);

//     let r_ready_prepare = r_receive_prepare;

//     // r_prepare_log.for_each(q!(|prepare| println!("r_verified_prepare: {:?}", prepare)));

//     let r_prepared = r_ready_prepare
//         .atomic(&replicas_tick)
//         .join(r_pre_prepare_log.clone().atomic(&replicas_tick))
//         .filter_map(q!(|(_n, (prepare, pre_prepare))| if (prepare.digest == pre_prepare.digest) {
//             Some(( 
//                 pre_prepare.n,
//                 Prepared {
//                 message: pre_prepare.message, // the message
//                 view_no: pre_prepare.view_no, // current view number
//                 n: pre_prepare.n,             // sequence number
//                 i: prepare.i,                 // replica number i
//             })
//             )
//         } else {
//             None
//         }
//         ));


//     let r_commit_ready_to_send = r_prepared.clone().map(q!(move |(n, prepared)| 
//         (
//         n, 
//         Commit {
//         view_no: prepared.view_no,                          // current view number
//         n: prepared.n,                                      // sequence number
//         digest: helper::calculate_hash(&prepared.message),  // currently set the digest to a u32
//         i: prepared.i,                                      // replica number i
//         replica_signature: helper::calculate_hash(&CLUSTER_SELF_ID), 
//         }
//         )
//     ));

//     // broadcast a COMMIT message
//     let r_receive_commit = r_commit_ready_to_send.broadcast_bincode_anonymous(&replicas);
//     // r_receive_commit.clone().for_each(q!(|(id, commit)| println!("commit: {id}, {:?}", commit)));

//     // count the commit message.  2f COMMIT messages from different backups matching the PREPARED
//     let (r_verified_commit_digest_quorum_reached, _) = collect_quorum(
//         r_receive_commit
//             .map(q!(move |(n, commit)| (
//                 (n, commit.digest),
//                 if commit.replica_signature != helper::calculate_hash(&CLUSTER_SELF_ID) {
//                     Ok(())
//                 } else {
//                     Err(())
//                 }
//             )))
//             .atomic(&replicas_tick),
//             2 * f, 
//             3 * f + 1,
//     );

//     // r_verified_commit_digest_quorum_reached.clone().for_each(q!(move |commit| println!(
//     //     "These operation will be executed on replica {}: {:?}", CLUSTER_SELF_ID.raw_id, 
//     //     commit
//     // )));

//     let r_committed = r_verified_commit_digest_quorum_reached.end_atomic()
//     .join(r_pre_prepare_log.clone()).filter_map(
//         q!(move |(n, (digest, pre_prepare))| if pre_prepare.digest == digest {
//             Some((
//                 n, 
//                 Committed {
//                 message: pre_prepare.message,
//                 view_no: pre_prepare.view_no,
//                 n: pre_prepare.n,
//                 i: CLUSTER_SELF_ID.raw_id
//             }))
//         } else {
//             None
//         })
//     );

//     let r_committed_log = r_committed.clone();

//     // r_committed.clone().for_each(q!(|(n, commited)|println!("{}, committed: {:?}", n, commited)));
//     let r_ready_send_reply = r_committed.map(q!(move |(n, committed)|
//         (n as usize, Some(committed.message))
//     ));

//     r_ready_send_reply
// }

