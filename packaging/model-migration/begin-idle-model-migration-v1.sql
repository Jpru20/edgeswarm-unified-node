-- Atomically withdraw an idle, authenticated desktop node from V2 admission.
-- No task, attempt, reservation, billing or provider desired-state mutation.
-- All W10/W12 create/claim operations use this same physical-device lock.
BEGIN;

CREATE OR REPLACE FUNCTION public.begin_idle_model_migration_v1(
    p_hardware_id text,
    p_provider_email text,
    p_worker text,
    p_app_version text,
    p_from_model text,
    p_to_model text
) RETURNS jsonb
LANGUAGE plpgsql
SECURITY INVOKER
SET search_path TO pg_catalog, public
AS $function$
DECLARE
    h public.node_heartbeats%ROWTYPE;
BEGIN
    IF p_hardware_id IS NULL OR p_hardware_id !~ '^[a-f0-9]{64}$'
       OR nullif(btrim(p_provider_email),'') IS NULL
       OR p_worker IS NULL OR p_worker !~ '^0x[a-fA-F0-9]{40}$'
       OR p_app_version IS NULL OR p_app_version !~ '^1[.]7[.][0-9]+$'
       OR p_from_model IS NULL OR length(p_from_model) NOT BETWEEN 2 AND 128
       OR p_to_model IS NULL OR length(p_to_model) NOT BETWEEN 2 AND 128
       OR p_from_model=p_to_model
    THEN RETURN jsonb_build_object('ok',false,'reason','invalid_identity'); END IF;

    PERFORM pg_advisory_xact_lock(240002,hashtext(p_hardware_id));
    SELECT * INTO h FROM public.node_heartbeats
      WHERE hardware_id=p_hardware_id FOR UPDATE;
    IF NOT FOUND OR lower(h.provider_email) IS DISTINCT FROM lower(p_provider_email)
       OR lower(h.worker) IS DISTINCT FROM lower(p_worker)
       OR h.app_version IS DISTINCT FROM p_app_version
       OR coalesce(lower(h.platform),'') NOT IN ('macos','windows','linux')
       OR h.metadata->>'unifiedProtocolVersion' IS DISTINCT FROM 'edgeswarm-unified-heartbeat-v1'
       OR h.status IS DISTINCT FROM 'online'
       OR h.model_status IS DISTINCT FROM 'ready'
       OR h.model_id IS DISTINCT FROM p_from_model
       OR h.last_seen <= clock_timestamp()-interval '30 seconds'
       OR coalesce(h.concurrency_limit,0)<1
    THEN RETURN jsonb_build_object('ok',false,'reason','node_not_fresh_owned_ready'); END IF;

    IF jsonb_typeof(h.current_task_ids) IS DISTINCT FROM 'array'
    THEN RETURN jsonb_build_object('ok',false,'reason','invalid_task_roster'); END IF;
    IF jsonb_array_length(h.current_task_ids)<>0
       OR EXISTS(SELECT 1 FROM public.v2_execution_attempts_v1 a
          WHERE a.hardware_id=p_hardware_id
            AND a.state IN ('prepared','leased','quiesced','finalizing'))
       OR EXISTS(SELECT 1 FROM public.tasks t
          WHERE t."assignedNodes" ? p_hardware_id
            AND t.status IN ('pending','processing','awaiting_quorum','finalizing'))
    THEN RETURN jsonb_build_object('ok',false,'reason','device_has_reserved_or_active_work'); END IF;

    IF NOT EXISTS(
      SELECT 1 FROM jsonb_array_elements(
        CASE WHEN jsonb_typeof(h.metadata->'modelCapacityV1')='array'
          THEN h.metadata->'modelCapacityV1' ELSE '[]'::jsonb END
      ) m
      WHERE m->>'selectedModel'=p_to_model AND m->>'status'='ready'
        AND m->>'capacityStatus'='certified'
        AND CASE WHEN m->>'certifiedConcurrency' ~ '^[0-9]{1,4}$'
             THEN (m->>'certifiedConcurrency')::int>0 ELSE false END
    ) THEN RETURN jsonb_build_object('ok',false,'reason','target_not_certified'); END IF;

    -- The node calls only after an empty successful poll, with no local workers.
    -- Ordinary online heartbeat from its healthy runtime restores admission.
    -- No expiring permit can accidentally re-admit a failed runtime.
    UPDATE public.node_heartbeats SET status='offline'
      WHERE hardware_id=p_hardware_id;
    RETURN jsonb_build_object('ok',true,'version','idle-model-migration-v1',
      'hardwareId',p_hardware_id,'fromModel',p_from_model,'toModel',p_to_model,
      'assignmentWithdrawn',true);
END;
$function$;

REVOKE ALL ON FUNCTION public.begin_idle_model_migration_v1(text,text,text,text,text,text) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.begin_idle_model_migration_v1(text,text,text,text,text,text) TO service_role;

COMMIT;
