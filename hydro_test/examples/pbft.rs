use hydro_deploy::Deployment;
use hydro_lang::{deploy::TrybuildHost, Cluster};
use hydro_test::cluster::{bench_client::Client, pbft::{CorePbft, PbftConfig}};

#[tokio::main]
async fn main() {
    let mut deployment = Deployment::new();
    let _localhost = deployment.Localhost();

    let builder = hydro_lang::FlowBuilder::new();
    let f = 1;
    let num_clients = 1;
    let num_clients_per_node = 100; // Change based on experiment between 1, 50, 100.
    let median_latency_window_size = 1000;
    let checkpoint_frequency = 1000; // Num log entries
    let i_am_leader_send_timeout = 5; // Sec
    let i_am_leader_check_timeout = 10; // Sec
    let i_am_leader_check_timeout_delay_multiplier = 15;

    // let clients:Cluster<'_, Client> = builder.cluster();
    let replicas = builder.cluster();
    // TODO: here, replicas declared inside pbft_bench.

    let clients = hydro_test::cluster::pbft_bench::pbft_bench(
        &builder,
        num_clients_per_node,
        median_latency_window_size,
        checkpoint_frequency,
        f,
        3 * f + 1,
        |replica_checkpoint| CorePbft {
            replicas: replicas.clone(),
            view_checkpoint: replica_checkpoint.broadcast_bincode(&replicas),
            pbft_config: PbftConfig {
                f,
                i_am_leader_send_timeout,
                i_am_leader_check_timeout,
                i_am_leader_check_timeout_delay_multiplier,
            },
        },
        replicas.clone()
    );

    let _rustflags = "-C opt-level=3 -C codegen-units=1 -C strip=none -C debuginfo=2 -C lto=off";

    let _nodes = builder
        .with_cluster(
            &replicas,
            (0..(3 * f + 1)).map(|_| TrybuildHost::new(deployment.Localhost())),
        )
        .with_cluster(
            &clients,
            (0..num_clients).map(|_| TrybuildHost::new(deployment.Localhost())),
        )
        .deploy(&mut deployment);

    deployment.deploy().await.unwrap();

    deployment.start().await.unwrap();

    tokio::signal::ctrl_c().await.unwrap();
}
