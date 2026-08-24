use a3s_runtime::contract::RuntimeInspection;

use super::*;

#[tokio::test]
async fn real_box_services_rebind_after_process_loss_without_runtime_or_sibling_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let generation = ServiceGeneration::new(
        temporary.path().join("package-v1"),
        PACKAGE_V1,
        MANIFEST_V1,
        7,
        PluginLifecycleAction::Install,
    );
    let process_store =
        QualificationProcessStore::new(temporary.path().join("processes/state.json"));
    let _cleanup = QualificationProcessCleanup::new(&process_store);
    let provider = QualificationProvider::start(
        temporary.path(),
        process_store.clone(),
        expected_services(&[&generation]),
    );
    let gateway_address = reserve_loopback_address();
    let paths = ComponentPaths::for_test(temporary.path());
    let gateway = start_gateway(gateway_address, &paths).await;
    let host = provider.lifecycle(&generation, gateway.clone()).await;

    prepare_generation(&host, &generation, '1', '2').await;
    let original_tool = service_receipt(
        &provider.bindings,
        &generation.intent,
        &generation.tool_plan,
    )
    .await;
    let original_mcp =
        service_receipt(&provider.bindings, &generation.intent, &generation.mcp_plan).await;
    let original_tool_process = process_store
        .identity_for_unit(&generation.tool_plan.spec().unit_id)
        .await
        .unwrap();
    let original_mcp_process = process_store
        .identity_for_unit(&generation.mcp_plan.spec().unit_id)
        .await
        .unwrap();

    process_store
        .terminate_unit_process(&generation.tool_plan.spec().unit_id)
        .await
        .unwrap();
    host.prepare_tool(&generation.intent, &generation.tool, &key('3'))
        .await
        .unwrap();
    let rebound_tool = service_receipt(
        &provider.bindings,
        &generation.intent,
        &generation.tool_plan,
    )
    .await;
    let rebound_tool_process = process_store
        .identity_for_unit(&generation.tool_plan.spec().unit_id)
        .await
        .unwrap();
    assert_rebound_receipt(&original_tool, &rebound_tool);
    assert_binding_removed(gateway.as_ref(), &original_tool);
    assert_replaced_process(&original_tool_process, &rebound_tool_process);
    assert_eq!(
        process_store
            .identity_for_unit(&generation.mcp_plan.spec().unit_id)
            .await
            .unwrap(),
        original_mcp_process
    );
    assert_tool_generation(gateway_address, &rebound_tool, 7, &generation.tool_plan).await;
    assert_mcp_generation(gateway_address, &original_mcp, 7, &generation.mcp_plan).await;

    process_store
        .terminate_unit_process(&generation.mcp_plan.spec().unit_id)
        .await
        .unwrap();
    host.prepare_mcp(&generation.intent, &generation.mcp, &key('4'))
        .await
        .unwrap();
    let rebound_mcp =
        service_receipt(&provider.bindings, &generation.intent, &generation.mcp_plan).await;
    let rebound_mcp_process = process_store
        .identity_for_unit(&generation.mcp_plan.spec().unit_id)
        .await
        .unwrap();
    assert_rebound_receipt(&original_mcp, &rebound_mcp);
    assert_binding_removed(gateway.as_ref(), &original_mcp);
    assert_replaced_process(&original_mcp_process, &rebound_mcp_process);
    assert_eq!(
        process_store
            .identity_for_unit(&generation.tool_plan.spec().unit_id)
            .await
            .unwrap(),
        rebound_tool_process
    );
    assert_eq!(process_store.active_records().await.unwrap().len(), 2);
    assert_tool_generation(gateway_address, &rebound_tool, 7, &generation.tool_plan).await;
    assert_mcp_generation(gateway_address, &rebound_mcp, 7, &generation.mcp_plan).await;

    host.stop_tool(&generation.intent, &generation.tool, &key('5'))
        .await
        .unwrap();
    host.remove_tool(&generation.intent, &generation.tool, &key('6'))
        .await
        .unwrap();
    host.stop_mcp(&generation.intent, &generation.mcp, &key('7'))
        .await
        .unwrap();
    host.remove_mcp(&generation.intent, &generation.mcp, &key('8'))
        .await
        .unwrap();

    for (plan, receipt) in [
        (&generation.tool_plan, &rebound_tool),
        (&generation.mcp_plan, &rebound_mcp),
    ] {
        assert_binding_removed(gateway.as_ref(), receipt);
        assert_no_receipt(&provider.bindings, &generation.intent, plan).await;
        assert!(matches!(
            provider.client.inspect(&plan.spec().unit_id).await.unwrap(),
            RuntimeInspection::NotFound { .. }
        ));
    }
    assert!(process_store.active_records().await.unwrap().is_empty());
    gateway.shutdown().await;
}

fn assert_rebound_receipt(
    original: &a3s_use::plugin_runtime::RuntimeServiceBindingReceipt,
    rebound: &a3s_use::plugin_runtime::RuntimeServiceBindingReceipt,
) {
    assert_ne!(rebound.endpoint_ref, original.endpoint_ref);
    assert_eq!(rebound.surface, original.surface);
    assert_eq!(rebound.unit_id, original.unit_id);
    assert_eq!(rebound.generation, original.generation);
    assert!(rebound.runtime_started_at_ms > original.runtime_started_at_ms);
    assert!(rebound.observation_revision > original.observation_revision);
}

fn assert_replaced_process(
    original: &QualificationProcessIdentity,
    rebound: &QualificationProcessIdentity,
) {
    assert_ne!(rebound.execution_id, original.execution_id);
    assert!(rebound.execution_generation > 0);
    assert_ne!(
        (rebound.pid, rebound.pid_start_time),
        (original.pid, original.pid_start_time)
    );
    assert_ne!(rebound.host_port, 0);
}
