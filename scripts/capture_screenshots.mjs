#!/usr/bin/env node
/**
 * Deterministic headless Chrome screenshot generator for the Relay Map MCP App UI.
 * Captures 9 high-DPI screenshots into site/assets/screenshots/ using 100% synthetic
 * multi-host, 5-agent-runtime fleet telemetry.
 */

import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';

const ROOT = path.resolve(import.meta.dirname, '..');
const HTML_PATH = path.join(ROOT, 'plugins', 'agent-relay', 'assets', 'relay-map.html');
const OUT_DIR = path.join(ROOT, 'site', 'assets', 'screenshots');
const CHROME_PATH = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

function buildSyntheticData(baseNow) {
  const m = (minsAgo) => baseNow - minsAgo * 60_000;
  const h = (hrsAgo) => baseNow - hrsAgo * 3_600_000;

  const hosts = [
    { name: 'studio', via: 'local', state: 'live', detail: 'macOS · local collector' },
    { name: 'macbook-pro', via: 'ssh', state: 'live', detail: 'ssh macbook-pro · 12 ms' },
    { name: 'gpu-worker', via: 'ssh', state: 'live', detail: 'ssh gpu-worker · 24 ms' },
    { name: 'ci-linux', via: 'ssh', state: 'live', detail: 'ssh ci-linux · 31 ms' },
  ];

  const pools = [
    { name: 'Payments', color: '#4cb8ff', isolated: true, auto: false },
    { name: 'Search', color: '#46f5a0', isolated: false, auto: false },
    { name: 'Platform', color: '#ff8a4c', isolated: false, auto: false },
  ];

  const agents = [
    {
      addr: 'claude:checkout-api@studio',
      id: 'c91f4a12-checkout-api',
      kind: 'claude',
      host: 'studio',
      pool: 'Payments',
      label: 'checkout-api',
      title: 'Harden idempotency keys and webhook retry backoff',
      focus: 'Verifying Stripe webhook signature replay tests across workers',
      last_text: 'All 42 idempotency and webhook retry tests pass; notifying payment-ledger.',
      cwd: '/workspace/checkout-api',
      model: 'claude-opus-4-6',
      state: 'busy',
      alive: true,
      started_ms: h(4.2),
      last_active_ms: m(0.3),
      cost_usd: 14.85,
      tokens: { input: 620_000, output: 118_000, cache_write: 410_000, cache_read: 9_800_000 },
      files: [
        { path: '/workspace/checkout-api/src/idempotency.ts', op: 'edited', count: 6, at_ms: m(1) },
        { path: '/workspace/checkout-api/src/webhook.ts', op: 'edited', count: 4, at_ms: m(4) },
        { path: '/workspace/checkout-api/tests/webhook.test.ts', op: 'created', count: 2, at_ms: m(9) },
      ],
      git: [
        { msg: 'feat: enforce idempotency key TTL', at_ms: m(18) },
        { msg: 'test: add webhook signature replay suite', at_ms: m(7) },
      ],
      skills: { 'message-session': 5, 'list-sessions': 3, 'relay-map': 1 },
      subagents: [
        { name: 'webhook-auditor', kind: 'research', status: 'running', task: 'Trace replay window edge cases', started_ms: m(12), last_ms: m(1) },
        { name: 'schema-checker', kind: 'verify', status: 'done', task: 'Validate OpenAPI webhook payload', started_ms: m(25), last_ms: m(14) },
      ],
    },
    {
      addr: 'codex:payment-ledger@macbook-pro',
      id: '019a88c1-payment-ledger',
      kind: 'codex',
      host: 'macbook-pro',
      pool: 'Payments',
      label: 'payment-ledger',
      title: 'Reconcile double-entry settlement batches in PostgreSQL',
      focus: 'Running settlement invariant checks against staging snapshot',
      last_text: 'Settlement batch #409 reconciled with zero drift.',
      cwd: '/workspace/payment-ledger',
      model: 'gpt-5.4',
      state: 'busy',
      alive: true,
      started_ms: h(3.5),
      last_active_ms: m(0.6),
      cost_usd: 11.40,
      tokens: { input: 540_000, output: 96_000, cache_write: 190_000, cache_read: 6_400_000 },
      files: [
        { path: '/workspace/payment-ledger/src/reconcile.rs', op: 'edited', count: 5, at_ms: m(2) },
        { path: '/workspace/payment-ledger/migrations/014_batch_idx.sql', op: 'created', count: 1, at_ms: m(15) },
      ],
      git: [{ msg: 'fix: lock settlement rows in batch order', at_ms: m(12) }],
      skills: { 'handle-relay-message': 4, 'message-session': 3 },
      subagents: [],
    },
    {
      addr: 'antigravity:checkout-ui@studio',
      id: 'ag-4402-checkout-ui',
      kind: 'antigravity',
      host: 'studio',
      pool: 'Payments',
      label: 'checkout-ui',
      title: 'Coordinate 3D Secure modal flow and accessibility checks',
      focus: 'Reviewing subagent DOM audit results for 3DS challenge frame',
      last_text: 'Keyboard trap resolved; waiting for checkout-api schema confirmation.',
      cwd: '/workspace/checkout-api',
      model: 'gemini-3.1-pro',
      state: 'busy',
      alive: true,
      started_ms: h(2.8),
      last_active_ms: m(0.8),
      cost_usd: 6.20,
      tokens: { input: 410_000, output: 74_000, cache_write: 140_000, cache_read: 4_200_000 },
      files: [
        { path: '/workspace/checkout-api/web/ThreeDSModal.tsx', op: 'edited', count: 3, at_ms: m(3) },
      ],
      git: [{ msg: 'a11y: trap focus inside 3DS challenge dialog', at_ms: m(20) }],
      skills: { 'relay-map': 2, 'message-session': 2 },
      subagents: [
        { name: 'dom-verifier', kind: 'browser', status: 'running', task: 'Verify ARIA focus ring in challenge modal', started_ms: m(8), last_ms: m(1) },
      ],
    },
    {
      addr: 'antigravity:checkout-a11y-worker@studio',
      id: 'ag-4403-a11y',
      kind: 'antigravity',
      host: 'studio',
      pool: 'Payments',
      parent: 'antigravity:checkout-ui@studio',
      label: 'checkout-a11y',
      title: 'Run Playwright keyboard navigation checks on 3DS modal',
      focus: 'Asserting Escape and Tab order across iframe boundaries',
      cwd: '/workspace/checkout-api',
      model: 'gemini-3.1-pro',
      state: 'busy',
      alive: true,
      started_ms: h(1.4),
      last_active_ms: m(1.1),
      cost_usd: 2.15,
      tokens: { input: 160_000, output: 31_000, cache_write: 50_000, cache_read: 1_200_000 },
      files: [{ path: '/workspace/checkout-api/e2e/3ds.spec.ts', op: 'created', count: 2, at_ms: m(6) }],
      git: [],
      skills: {},
      subagents: [],
    },
    {
      addr: 'antigravity:checkout-perf-worker@studio',
      id: 'ag-4404-perf',
      kind: 'antigravity',
      host: 'studio',
      pool: 'Payments',
      parent: 'antigravity:checkout-ui@studio',
      label: 'checkout-perf',
      title: 'Measure checkout bundle size and Lighthouse CLS score',
      focus: 'Bundle delta is -14 KB gzipped after tree-shaking',
      cwd: '/workspace/checkout-api',
      model: 'gemini-3.1-pro',
      state: 'idle',
      alive: true,
      started_ms: h(1.3),
      last_active_ms: m(1.4),
      cost_usd: 1.80,
      tokens: { input: 120_000, output: 24_000, cache_write: 40_000, cache_read: 980_000 },
      files: [],
      git: [],
      skills: {},
      subagents: [],
    },
    {
      addr: 'opencode:fraud-rules@ci-linux',
      id: 'oc-7810-fraud',
      kind: 'opencode',
      host: 'ci-linux',
      pool: 'Payments',
      label: 'fraud-rules',
      title: 'Tune velocity rule thresholds on anonymized test fixtures',
      focus: 'Waiting for operator approval on threshold bump',
      last_text: 'Proposed threshold change reduces false positives by 18%.',
      cwd: '/workspace/payment-ledger',
      model: 'claude-sonnet-4-6',
      state: 'idle',
      alive: true,
      started_ms: h(3.0),
      last_active_ms: m(18),
      cost_usd: 4.95,
      tokens: { input: 280_000, output: 51_000, cache_write: 95_000, cache_read: 2_300_000 },
      files: [{ path: '/workspace/payment-ledger/rules/velocity.yaml', op: 'edited', count: 3, at_ms: m(21) }],
      git: [],
      skills: { 'handle-relay-message': 2 },
      subagents: [],
    },
    {
      addr: 'codex:search-ranker@gpu-worker',
      id: '019a9102-search-ranker',
      kind: 'codex',
      host: 'gpu-worker',
      pool: 'Search',
      label: 'search-ranker',
      title: 'Benchmark hybrid BM25 + dense reranker on 50k queries',
      focus: 'Evaluating NDCG@10 across rerank_k=40 checkpoint',
      last_text: 'NDCG@10 improved from 0.742 to 0.789 (+4.7 pts) at 11.2 ms p95.',
      cwd: '/workspace/search-ranker',
      model: 'gpt-5.4',
      state: 'busy',
      alive: true,
      started_ms: h(5.1),
      last_active_ms: m(0.4),
      cost_usd: 18.60,
      tokens: { input: 890_000, output: 164_000, cache_write: 260_000, cache_read: 11_400_000 },
      files: [
        { path: '/workspace/search-ranker/eval/benchmark.py', op: 'edited', count: 7, at_ms: m(2) },
        { path: '/workspace/search-ranker/src/fusion.rs', op: 'edited', count: 4, at_ms: m(8) },
      ],
      git: [{ msg: 'perf: fuse sparse and dense scores in SIMD loop', at_ms: m(11) }],
      skills: { 'message-session': 4, 'list-sessions': 2 },
      subagents: [
        { name: 'eval-runner', kind: 'verify', status: 'running', task: 'Run 50k query latency sweep', started_ms: m(15), last_ms: m(1) },
      ],
    },
    {
      addr: 'claude:vector-index@gpu-worker',
      id: 'c882d091-vector-index',
      kind: 'claude',
      host: 'gpu-worker',
      pool: 'Search',
      label: 'vector-index',
      title: 'Build HNSW quantized index shards with zero-copy mmap',
      focus: 'Compacting shard-04 into int8 quantized segment',
      last_text: 'Shard-04 compacted in 19.4 s; RAM footprint down 62%.',
      cwd: '/workspace/vector-index',
      model: 'claude-opus-4-6',
      state: 'busy',
      alive: true,
      started_ms: h(4.0),
      last_active_ms: m(0.9),
      cost_usd: 16.30,
      tokens: { input: 710_000, output: 132_000, cache_write: 380_000, cache_read: 8_900_000 },
      files: [
        { path: '/workspace/vector-index/src/hnsw.rs', op: 'edited', count: 8, at_ms: m(3) },
        { path: '/workspace/vector-index/src/mmap.rs', op: 'written', count: 3, at_ms: m(10) },
      ],
      git: [{ msg: 'feat: int8 scalar quantization for HNSW layers', at_ms: m(14) }],
      skills: { 'message-session': 3, 'handle-relay-message': 3 },
      subagents: [],
    },
    {
      addr: 'pi:query-parser@macbook-pro',
      id: 'pi-3109-query-parser',
      kind: 'pi',
      host: 'macbook-pro',
      pool: 'Search',
      label: 'query-parser',
      title: 'Add facet filter grammar for date ranges and negative terms',
      focus: 'Testing quoted phrase + negation AST precedence',
      last_text: 'AST parser fuzz suite passed 10,000 cases.',
      cwd: '/workspace/search-ranker',
      model: 'claude-sonnet-4-6',
      state: 'busy',
      alive: true,
      started_ms: h(2.4),
      last_active_ms: m(1.2),
      cost_usd: 4.10,
      tokens: { input: 240_000, output: 46_000, cache_write: 82_000, cache_read: 1_900_000 },
      files: [{ path: '/workspace/search-ranker/src/query_ast.ts', op: 'edited', count: 5, at_ms: m(5) }],
      git: [{ msg: 'feat: support -exclude and quoted ranges', at_ms: m(19) }],
      skills: { 'handle-relay-message': 2 },
      subagents: [],
    },
    {
      addr: 'opencode:eval-harness@ci-linux',
      id: 'oc-5541-eval',
      kind: 'opencode',
      host: 'ci-linux',
      pool: 'Search',
      label: 'eval-harness',
      title: 'Compare baseline vs hybrid reranker recall regressions',
      focus: 'Awaiting golden dataset checksum from search-ranker',
      last_text: 'Golden fixture manifest loaded; ready for batch run.',
      cwd: '/workspace/eval-runner',
      model: 'claude-sonnet-4-6',
      state: 'idle',
      alive: true,
      started_ms: h(2.1),
      last_active_ms: m(14),
      cost_usd: 2.95,
      tokens: { input: 175_000, output: 33_000, cache_write: 60_000, cache_read: 1_450_000 },
      files: [{ path: '/workspace/eval-runner/run_eval.py', op: 'edited', count: 2, at_ms: m(16) }],
      git: [],
      skills: { 'list-sessions': 1 },
      subagents: [],
    },
    {
      addr: 'claude:auth-gateway@studio',
      id: 'c771a004-auth-gateway',
      kind: 'claude',
      host: 'studio',
      pool: 'Platform',
      label: 'auth-gateway',
      title: 'Rotate JWKS signing keys and verify zero-downtime rollover',
      focus: 'Validating dual-key JWT verification in staging middleware',
      last_text: 'Both kid=v4 and kid=v5 verify cleanly; ready to publish JWKS.',
      cwd: '/workspace/auth-gateway',
      model: 'claude-opus-4-6',
      state: 'busy',
      alive: true,
      started_ms: h(3.8),
      last_active_ms: m(0.5),
      cost_usd: 12.15,
      tokens: { input: 510_000, output: 91_000, cache_write: 290_000, cache_read: 6_800_000 },
      files: [
        { path: '/workspace/auth-gateway/src/jwks.go', op: 'edited', count: 4, at_ms: m(3) },
        { path: '/workspace/auth-gateway/src/jwks_test.go', op: 'edited', count: 3, at_ms: m(6) },
      ],
      git: [{ msg: 'sec: support overlapping JWKS key grace period', at_ms: m(16) }],
      skills: { 'message-session': 3, 'relay-setup': 1 },
      subagents: [],
    },
    {
      addr: 'codex:web-portal@macbook-pro',
      id: '019a7740-web-portal',
      kind: 'codex',
      host: 'macbook-pro',
      pool: 'Platform',
      label: 'web-portal',
      title: 'Migrate session dashboard to Server-Sent Events stream',
      focus: 'Checking reconnect backoff under packet loss',
      last_text: 'SSE stream reconnects within 400 ms with Last-Event-ID replay.',
      cwd: '/workspace/web-portal',
      model: 'gpt-5.4',
      state: 'busy',
      alive: true,
      started_ms: h(2.9),
      last_active_ms: m(1.0),
      cost_usd: 7.45,
      tokens: { input: 360_000, output: 68_000, cache_write: 120_000, cache_read: 3_700_000 },
      files: [{ path: '/workspace/web-portal/src/sse.ts', op: 'edited', count: 5, at_ms: m(4) }],
      git: [{ msg: 'feat: SSE Last-Event-ID resume support', at_ms: m(22) }],
      skills: { 'handle-relay-message': 3 },
      subagents: [],
    },
    {
      addr: 'pi:schema-linter@ci-linux',
      id: 'pi-9012-schema-linter',
      kind: 'pi',
      host: 'ci-linux',
      pool: 'Platform',
      label: 'schema-linter',
      title: 'Auto-fix OpenAPI breaking change diff in CI pipeline',
      focus: 'Retrying schema generation after circular ref conflict',
      last_text: 'Still hitting circular $ref on PolicyNode; retrying patch.',
      cwd: '/workspace/auth-gateway',
      model: 'claude-sonnet-4-6',
      state: 'busy',
      alive: true,
      started_ms: h(1.8),
      last_active_ms: m(2.2),
      cost_usd: 3.40,
      tokens: { input: 210_000, output: 42_000, cache_write: 70_000, cache_read: 1_600_000 },
      files: [{ path: '/workspace/auth-gateway/api/openapi.yaml', op: 'edited', count: 44, at_ms: m(2.2) }],
      git: [],
      skills: { 'handle-relay-message': 5 },
      subagents: [],
    },
    {
      addr: 'antigravity:deploy-coordinator@studio',
      id: 'ag-8120-deploy',
      kind: 'antigravity',
      host: 'studio',
      pool: 'Platform',
      label: 'deploy-coordinator',
      title: 'Orchestrate canary rollout across Envoy Gateway pods',
      focus: 'Waiting on canary health check confirmation',
      last_text: 'Canary 10% traffic split healthy; awaiting promotion.',
      cwd: '/workspace/web-portal',
      model: 'gemini-3.1-pro',
      state: 'idle',
      alive: true,
      started_ms: h(2.2),
      last_active_ms: m(16),
      cost_usd: 3.85,
      tokens: { input: 260_000, output: 48_000, cache_write: 88_000, cache_read: 2_100_000 },
      files: [{ path: '/workspace/web-portal/deploy/canary.yaml', op: 'edited', count: 2, at_ms: m(18) }],
      git: [],
      skills: { 'relay-map': 1, 'message-session': 2 },
      subagents: [
        { name: 'canary-probe', kind: 'verify', status: 'done', task: 'Verify HTTP 200 on /healthz', started_ms: m(30), last_ms: m(17) },
      ],
    },
    {
      addr: 'codex:legacy-migrator@ci-linux',
      id: '019a6611-legacy-migrator',
      kind: 'codex',
      host: 'ci-linux',
      pool: 'Platform',
      label: 'legacy-migrator',
      title: 'Backfill audit log archive partitions to object storage',
      focus: 'Blocked on staging bucket lock acquisition',
      last_text: 'Waiting for archive lock release from worker-02...',
      cwd: '/workspace/auth-gateway',
      model: 'gpt-5.4',
      state: 'busy',
      alive: true,
      started_ms: h(3.1),
      last_active_ms: m(23),
      cost_usd: 5.10,
      tokens: { input: 290_000, output: 38_000, cache_write: 60_000, cache_read: 1_800_000 },
      files: [],
      git: [],
      skills: {},
      subagents: [],
    },
    {
      addr: 'claude:docs-sync@macbook-pro',
      id: 'c330b112-docs-sync',
      kind: 'claude',
      host: 'macbook-pro',
      pool: null,
      label: 'docs-sync',
      title: 'Sync API reference docs with updated MCP tool schemas',
      focus: 'Checking example curl snippets against reference.md',
      last_text: 'Updated reference docs and verified markdown links.',
      cwd: '/workspace/web-portal',
      model: 'claude-haiku-4-5',
      state: 'idle',
      alive: false,
      started_ms: h(6.0),
      last_active_ms: m(48),
      cost_usd: 0.85,
      tokens: { input: 95_000, output: 22_000, cache_write: 30_000, cache_read: 620_000 },
      files: [{ path: '/workspace/web-portal/docs/api.md', op: 'edited', count: 2, at_ms: m(50) }],
      git: [],
      skills: {},
      subagents: [],
    },
    {
      addr: 'operator:relay-map@studio',
      id: 'relay-map-operator',
      kind: 'operator',
      host: 'studio',
      pool: null,
      label: 'Relay Map',
      title: 'Relay Map Operator',
      focus: 'Monitoring 16 live agents across 4 hosts',
      cwd: '',
      model: '',
      state: 'busy',
      alive: true,
      started_ms: h(6.0),
      last_active_ms: m(0.1),
      tokens: { input: 0, output: 0, cache_write: 0, cache_read: 0 },
      files: [],
      git: [],
      skills: {},
      subagents: [],
    },
  ];

  const messages = [
    {
      id: 'msg-01',
      from: 'claude:checkout-api@studio',
      to: 'codex:payment-ledger@macbook-pro',
      from_agent: 'claude:checkout-api@studio',
      to_agent: 'codex:payment-ledger@macbook-pro',
      subject: 'Idempotency key TTL schema update',
      body: 'Updated idempotency_keys.expires_at to 24h UTC and added replay guard tests. Can you verify settlement batch #409 handles duplicate webhook deliveries cleanly?',
      sent_ms: m(42),
      bytes: 218,
      source: 'relay',
    },
    {
      id: 'msg-02',
      from: 'codex:payment-ledger@macbook-pro',
      to: 'claude:checkout-api@studio',
      from_agent: 'codex:payment-ledger@macbook-pro',
      to_agent: 'claude:checkout-api@studio',
      subject: 'Re: Idempotency key TTL schema update',
      body: 'Verified against staging snapshot: batch #409 deduplicates replay keys with zero ledger drift. Migration 014_batch_idx.sql is ready.',
      sent_ms: m(36),
      bytes: 184,
      source: 'relay',
    },
    {
      id: 'msg-03',
      from: 'claude:checkout-api@studio',
      to: 'antigravity:checkout-ui@studio',
      from_agent: 'claude:checkout-api@studio',
      to_agent: 'antigravity:checkout-ui@studio',
      subject: '3DS challenge callback payload ready',
      body: 'POST /v2/checkout/3ds/complete now returns status=verified with an idempotency token. Ready for UI integration.',
      sent_ms: m(29),
      bytes: 162,
      source: 'relay',
    },
    {
      id: 'msg-04',
      from: 'antigravity:checkout-ui@studio',
      to: 'antigravity:checkout-a11y-worker@studio',
      from_agent: 'antigravity:checkout-ui@studio',
      to_agent: 'antigravity:checkout-a11y-worker@studio',
      subject: 'Run 3DS modal keyboard audit',
      body: 'Verify Tab focus trap and Escape dismissal inside ThreeDSModal.tsx.',
      sent_ms: m(24),
      bytes: 112,
      source: 'swarm',
    },
    {
      id: 'msg-05',
      from: 'antigravity:checkout-ui@studio',
      to: 'antigravity:checkout-perf-worker@studio',
      from_agent: 'antigravity:checkout-ui@studio',
      to_agent: 'antigravity:checkout-perf-worker@studio',
      subject: 'Check checkout bundle delta',
      body: 'Measure gzip bundle size after tree-shaking unused locales.',
      sent_ms: m(22),
      bytes: 98,
      source: 'swarm',
    },
    {
      id: 'msg-06',
      from: 'codex:search-ranker@gpu-worker',
      to: 'claude:vector-index@gpu-worker',
      from_agent: 'codex:search-ranker@gpu-worker',
      to_agent: 'claude:vector-index@gpu-worker',
      subject: 'Request int8 HNSW shard-04 mmap path',
      body: 'Need the compacted shard-04 path and recall@10 benchmark numbers to run hybrid BM25 + dense fusion sweep.',
      sent_ms: m(19),
      bytes: 158,
      source: 'relay',
    },
    {
      id: 'msg-07',
      from: 'claude:vector-index@gpu-worker',
      to: 'codex:search-ranker@gpu-worker',
      from_agent: 'claude:vector-index@gpu-worker',
      to_agent: 'codex:search-ranker@gpu-worker',
      subject: 'Re: Request int8 HNSW shard-04 mmap path',
      body: 'Shard-04 compacted at /data/shards/hnsw-04.int8.mmap (RAM -62%, recall@10 = 0.964). Ready for fusion sweep.',
      sent_ms: m(15),
      bytes: 171,
      source: 'relay',
    },
    {
      id: 'msg-08',
      from: 'pi:query-parser@macbook-pro',
      to: 'codex:search-ranker@gpu-worker',
      from_agent: 'pi:query-parser@macbook-pro',
      to_agent: 'codex:search-ranker@gpu-worker',
      subject: 'Negated term AST filters merged',
      body: 'query_ast.ts now emits structured MUST_NOT clauses for -term syntax; all 10k fuzz cases green.',
      sent_ms: m(13),
      bytes: 146,
      source: 'relay',
    },
    {
      id: 'msg-09',
      from: 'claude:auth-gateway@studio',
      to: 'codex:web-portal@macbook-pro',
      from_agent: 'claude:auth-gateway@studio',
      to_agent: 'codex:web-portal@macbook-pro',
      subject: 'JWKS dual-key rollover active in staging',
      body: 'kid=v5 added alongside kid=v4 on /.well-known/jwks.json. Please verify SSE reconnect keeps bearer refresh smooth.',
      sent_ms: m(11),
      bytes: 164,
      source: 'relay',
    },
    {
      id: 'msg-10',
      from: 'codex:web-portal@macbook-pro',
      to: 'antigravity:deploy-coordinator@studio',
      from_agent: 'codex:web-portal@macbook-pro',
      to_agent: 'antigravity:deploy-coordinator@studio',
      subject: 'SSE reconnect verified under packet loss',
      body: 'Last-Event-ID resume recovers in 400 ms. Safe to promote canary after JWKS check.',
      sent_ms: m(9),
      bytes: 138,
      source: 'relay',
    },
    {
      id: 'msg-11',
      from: 'claude:checkout-api@studio',
      to: 'claude:auth-gateway@studio',
      from_agent: 'claude:checkout-api@studio',
      to_agent: 'claude:auth-gateway@studio',
      subject: 'Cross-pool check: webhook HMAC scope',
      body: 'Confirming Payments isolated service account scope is restricted to /v2/webhooks/verify.',
      sent_ms: m(7),
      bytes: 142,
      source: 'relay',
    },
    {
      id: 'msg-12',
      from: 'codex:search-ranker@gpu-worker',
      to: 'opencode:eval-harness@ci-linux',
      from_agent: 'codex:search-ranker@gpu-worker',
      to_agent: 'opencode:eval-harness@ci-linux',
      subject: 'New NDCG@10 baseline (+4.7 pts)',
      body: 'Hybrid rerank_k=40 hit 0.789 NDCG@10 at 11.2 ms p95. Trigger full regression suite on ci-linux.',
      sent_ms: m(5),
      bytes: 156,
      source: 'relay',
    },
    {
      id: 'msg-13',
      from: 'claude:checkout-api@studio',
      to: 'codex:payment-ledger@macbook-pro',
      from_agent: 'claude:checkout-api@studio',
      to_agent: 'codex:payment-ledger@macbook-pro',
      subject: 'Run final end-to-end settlement dry run',
      body: 'All 42 webhook replay tests passed on studio. Kick off the final end-to-end settlement dry run on macbook-pro and send the summary.',
      sent_ms: m(3),
      bytes: 194,
      source: 'relay',
    },
    {
      id: 'msg-14',
      from: 'codex:payment-ledger@macbook-pro',
      to: 'claude:checkout-api@studio',
      from_agent: 'codex:payment-ledger@macbook-pro',
      to_agent: 'claude:checkout-api@studio',
      subject: 'Re: Run final end-to-end settlement dry run',
      body: 'Dry run complete: 1,240 transactions settled in 1.8 s with 0 discrepancies. Ready to tag release.',
      sent_ms: m(1),
      bytes: 172,
      source: 'relay',
    },
    {
      id: 'msg-15',
      from: 'operator:relay-map@studio',
      to: 'opencode:fraud-rules@ci-linux',
      from_agent: 'operator:relay-map@studio',
      to_agent: 'opencode:fraud-rules@ci-linux',
      subject: 'Status check',
      body: 'Checking fleet readiness before canary promotion.',
      sent_ms: m(0.5),
      bytes: 94,
      source: 'broadcast',
    },
  ];

  // Add ping-pong messages between pi:schema-linter@ci-linux and claude:auth-gateway@studio?
  // Note: schema-linter already triggers looping via 44 edits to openapi.yaml in the last 2.2 min!
  // Let's also give a couple more recent timestamps in the last 24h for timeline density.
  for (let i = 0; i < 18; i++) {
    messages.unshift({
      id: `hist-${i}`,
      from: i % 2 === 0 ? 'claude:checkout-api@studio' : 'codex:search-ranker@gpu-worker',
      to: i % 2 === 0 ? 'antigravity:checkout-ui@studio' : 'pi:query-parser@macbook-pro',
      from_agent: i % 2 === 0 ? 'claude:checkout-api@studio' : 'codex:search-ranker@gpu-worker',
      to_agent: i % 2 === 0 ? 'antigravity:checkout-ui@studio' : 'pi:query-parser@macbook-pro',
      subject: 'Checkpoint sync',
      body: 'Incremental build and test checkpoint synced.',
      sent_ms: m(90 + i * 35),
      bytes: 120 + i * 7,
      source: 'relay',
    });
  }
  messages.sort((a, b) => a.sent_ms - b.sent_ms);

  const prefs = {
    pins: ['claude:checkout-api@studio', 'codex:search-ranker@gpu-worker'],
    levels: ['host', 'project', 'directory', 'task'],
    color_by: 'kind',
    window_hours: 24,
    show_unpooled: true,
    show_swarm: true,
    expand_swarms: false,
  };

  const transcript = {
    addr: 'codex:search-ranker@gpu-worker',
    source: 'codex',
    bytes: 48200,
    notes: [],
    entries: [
      {
        role: 'user',
        t: m(22),
        text: 'Benchmark the hybrid BM25 + int8 HNSW dense reranker at rerank_k=40 on the 50k query set and compare p95 latency against baseline.',
      },
      {
        role: 'thinking',
        t: m(21),
        text: 'Let me inspect eval/benchmark.py and src/fusion.rs to verify SIMD score fusion before launching the 50k sweep on gpu-worker.',
      },
      {
        role: 'tool',
        name: 'read_file',
        t: m(20),
        text: '{"path":"/workspace/search-ranker/src/fusion.rs","lines":"1-120"}',
      },
      {
        role: 'result',
        t: m(19),
        text: 'pub fn fuse_scores(sparse: &[f32], dense: &[f32], alpha: f32) -> Vec<f32> { ... }',
      },
      {
        role: 'tool',
        name: 'run_command',
        t: m(16),
        text: 'python3 eval/benchmark.py --queries 50000 --rerank-k 40 --shard /data/shards/hnsw-04.int8.mmap',
      },
      {
        role: 'result',
        t: m(8),
        text: 'Queries: 50000 | Baseline NDCG@10: 0.742 | Hybrid NDCG@10: 0.789 (+4.7 pts) | p95 latency: 11.2 ms',
      },
      {
        role: 'assistant',
        t: m(5),
        text: 'Benchmark complete on gpu-worker:\n- NDCG@10 improved from 0.742 to 0.789 (+4.7 pts)\n- p95 latency is 11.2 ms with int8 HNSW shard-04\n- Sent the updated baseline to opencode:eval-harness@ci-linux via relay_send.',
      },
    ],
  };

  return { hosts, pools, agents, messages, prefs, transcript };
}

function makeSnapshotAtStep(baseData, stepIdx, totalSteps, baseNow) {
  const frac = stepIdx / totalSteps;
  const stepMs = baseNow - (totalSteps - stepIdx) * 60_000;
  const agents = baseData.agents.map((a, idx) => {
    if (a.kind === 'operator') return { ...a, last_active_ms: stepMs };
    const isHot = idx < 4 || a.kind === 'codex' || a.kind === 'claude';
    const scale = isHot ? 0.65 + 0.35 * frac : 0.9 + 0.1 * frac;
    return {
      ...a,
      last_active_ms: Math.min(a.last_active_ms, stepMs),
      tokens: {
        input: Math.round(a.tokens.input * scale),
        output: Math.round(a.tokens.output * scale),
        cache_write: Math.round(a.tokens.cache_write * scale),
        cache_read: Math.round(a.tokens.cache_read * scale),
      },
      cost_usd: typeof a.cost_usd === 'number' ? Number((a.cost_usd * scale).toFixed(2)) : undefined,
    };
  });
  return {
    self_host: 'studio',
    hosts: baseData.hosts,
    pools: baseData.pools,
    agents,
    messages: baseData.messages.filter((msg) => msg.sent_ms <= stepMs),
    prefs: baseData.prefs,
  };
}

function buildHostHtml(baseData, baseNow) {
  const initialSnap = makeSnapshotAtStep(baseData, 0, 29, baseNow);
  const payload = JSON.stringify({ initialSnap, transcript: baseData.transcript, agents: baseData.agents });
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<style>
  html, body, iframe { margin: 0; padding: 0; width: 100%; height: 100%; border: 0; overflow: hidden; background: #03060f; }
</style>
</head>
<body>
<iframe id="app" src="/relay-map.html"></iframe>
<script>
const DATA = ${payload};
let currentSnap = DATA.initialSnap;
const iframe = document.getElementById('app');

window.__setSnapshot = (snap) => {
  currentSnap = snap;
  iframe.contentWindow.postMessage({
    jsonrpc: '2.0',
    method: 'ui/notifications/tool-result',
    params: {
      content: [{ type: 'text', text: 'Relay Map snapshot' }],
      structuredContent: currentSnap,
    },
  }, '*');
};

window.addEventListener('message', (event) => {
  const msg = event.data;
  if (!msg || typeof msg !== 'object') return;
  const reply = (result) => {
    iframe.contentWindow.postMessage({ jsonrpc: '2.0', id: msg.id, result }, '*');
  };
  if (msg.method === 'ui/initialize') {
    reply({
      protocolVersion: '2026-01-26',
      hostInfo: { name: 'Agent Relay Showcase', version: '1.1.0' },
      hostCapabilities: { openLinks: {}, message: {} },
      hostContext: { theme: 'dark', containerDimensions: { width: 1520, height: 950 } },
    });
  } else if (msg.method === 'ui/notifications/initialized') {
    window.__setSnapshot(currentSnap);
  } else if (msg.method === 'tools/call') {
    const name = msg.params?.name;
    const args = msg.params?.arguments || {};
    if (name === 'relay_map_poll') {
      reply({ content: [{ type: 'text', text: 'ok' }], structuredContent: currentSnap });
    } else if (name === 'relay_map_prefs') {
      reply({ content: [{ type: 'text', text: 'saved' }], structuredContent: { ok: true, prefs: { ...currentSnap.prefs, ...(args.patch || {}) } } });
    } else if (name === 'relay_transcript') {
      reply({ content: [{ type: 'text', text: 'transcript' }], structuredContent: DATA.transcript });
    } else if (name === 'relay_broadcast') {
      const scope = String(args.scope || 'all');
      const targets = DATA.agents
        .filter((a) => a.kind !== 'operator' && a.alive && (scope === 'all' || scope === 'pool:' + a.pool || scope === 'agent:' + a.addr))
        .map((a) => a.addr);
      const skipped = DATA.agents
        .filter((a) => a.kind !== 'operator' && !targets.includes(a.addr))
        .map((a) => a.addr + ' (idle or outside scope)');
      reply({
        content: [{ type: 'text', text: 'preview' }],
        structuredContent: { targets, skipped, results: [] },
      });
    } else {
      reply({ content: [{ type: 'text', text: 'ok' }], structuredContent: {} });
    }
  } else if (msg.id !== undefined) {
    reply({});
  }
});
</script>
</body>
</html>`;
}

class CdpClient {
  constructor(wsUrl) {
    this.ws = new WebSocket(wsUrl);
    this.nextId = 1;
    this.pending = new Map();
  }
  async connect() {
    await new Promise((resolve, reject) => {
      this.ws.addEventListener('open', resolve, { once: true });
      this.ws.addEventListener('error', reject, { once: true });
    });
    this.ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) reject(new Error(msg.error.message || JSON.stringify(msg.error)));
        else resolve(msg.result);
      }
    });
  }
  send(method, params = {}) {
    const id = this.nextId++;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
  }
  async evalTop(expr) {
    const res = await this.send('Runtime.evaluate', {
      expression: `(async () => { ${expr} })()`,
      awaitPromise: true,
      returnByValue: true,
    });
    if (res.exceptionDetails) {
      throw new Error(res.exceptionDetails.text || JSON.stringify(res.exceptionDetails));
    }
    return res.result?.value;
  }
  async evalApp(expr) {
    const res = await this.send('Runtime.evaluate', {
      expression: `(async () => { const win = document.getElementById('app').contentWindow; const doc = win.document; ${expr} })()`,
      awaitPromise: true,
      returnByValue: true,
    });
    if (res.exceptionDetails) {
      throw new Error(res.exceptionDetails.text || JSON.stringify(res.exceptionDetails));
    }
    return res.result?.value;
  }
  async clickApp(selector) {
    const rect = await this.evalApp(`
      const el = doc.querySelector(${JSON.stringify(selector)});
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    `);
    if (!rect) throw new Error(`Element not found in app: ${selector}`);
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: rect.x, y: rect.y });
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: rect.x, y: rect.y, button: 'left', clickCount: 1 });
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: rect.x, y: rect.y, button: 'left', clickCount: 1 });
  }
  async screenshot(destPath) {
    const { data } = await this.send('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync(destPath, Buffer.from(data, 'base64'));
    execFileSync('/usr/bin/sips', ['-Z', '1920', destPath], { stdio: 'ignore' });
  }
  close() {
    this.ws.close();
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  if (!fs.existsSync(HTML_PATH)) {
    throw new Error(`Missing bundled MCP App HTML: ${HTML_PATH}`);
  }
  if (!fs.existsSync(CHROME_PATH)) {
    throw new Error(`Missing Google Chrome at ${CHROME_PATH}`);
  }

  fs.mkdirSync(OUT_DIR, { recursive: true });
  const baseNow = Date.now();
  const baseData = buildSyntheticData(baseNow);
  const hostHtml = buildHostHtml(baseData, baseNow);
  const relayMapHtml = fs.readFileSync(HTML_PATH, 'utf8');

  const server = http.createServer((req, res) => {
    if (req.url === '/relay-map.html') {
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
      res.end(relayMapHtml);
    } else {
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
      res.end(hostHtml);
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  const baseUrl = `http://127.0.0.1:${port}`;

  const profileDir = fs.mkdtempSync(path.join(os.tmpdir(), 'relay-shots-'));
  const chrome = spawn(
    CHROME_PATH,
    [
      '--headless=new',
      '--disable-gpu-sandbox',
      '--no-first-run',
      '--no-default-browser-check',
      '--remote-debugging-port=0',
      '--window-size=1520,950',
      '--force-device-scale-factor=2',
      '--hide-scrollbars',
      `--user-data-dir=${profileDir}`,
      'about:blank',
    ],
    { stdio: ['ignore', 'ignore', 'pipe'] },
  );

  let wsUrl = '';
  try {
    wsUrl = await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('Timed out waiting for Chrome DevTools WS URL')), 15000);
      chrome.stderr.on('data', (chunk) => {
        const match = String(chunk).match(/DevTools listening on (ws:\/\/[^\s]+)/);
        if (match) {
          clearTimeout(timeout);
          resolve(match[1]);
        }
      });
      chrome.once('exit', (code) => {
        clearTimeout(timeout);
        reject(new Error(`Chrome exited early with code ${code}`));
      });
    });

    const targetsRes = await fetch(wsUrl.replace(/^ws:/, 'http:').replace(/\/devtools\/browser\/.*$/, '/json/list'));
    const targets = await targetsRes.json();
    const pageTarget = targets.find((t) => t.type === 'page');
    if (!pageTarget?.webSocketDebuggerUrl) {
      throw new Error('Could not find Chrome page target');
    }

    const cdp = new CdpClient(pageTarget.webSocketDebuggerUrl);
    await cdp.connect();
    await cdp.send('Page.enable');
    await cdp.send('Runtime.enable');
    await cdp.send('Emulation.setDeviceMetricsOverride', {
      width: 1520,
      height: 950,
      deviceScaleFactor: 2,
      mobile: false,
    });
    await cdp.send('Page.addScriptToEvaluateOnNewDocument', {
      source: `(() => {
        const _origNow = Date.now.bind(Date);
        if (window.top === window) window.__fakeNow = ${baseNow - 29 * 60_000};
        Date.now = () => {
          try {
            if (window.top && window.top.__fakeNow !== null && window.top.__fakeNow !== undefined) {
              return window.top.__fakeNow;
            }
          } catch {}
          return _origNow();
        };
      })();`,
    });

    await cdp.send('Page.navigate', { url: `${baseUrl}/host.html` });
    await sleep(1000);

    // Step through 30 minutes of snapshots so burn rates and sparklines populate.
    for (let step = 1; step <= 29; step++) {
      const snap = makeSnapshotAtStep(baseData, step, 29, baseNow);
      const fakeMs = baseNow - (29 - step) * 60_000;
      await cdp.evalTop(`
        window.__fakeNow = ${fakeMs};
        window.__setSnapshot(${JSON.stringify(snap)});
      `);
      await sleep(35);
    }
    await cdp.evalTop(`window.__fakeNow = null;`);
    await sleep(2200);

    // 1. Orbital fleet map with live replay pulses in flight.
    await cdp.evalApp(`
      doc.getElementById('btn-fit').click();
      doc.getElementById('btn-replay').click();
    `);
    await sleep(2200);
    await cdp.screenshot(path.join(OUT_DIR, '01-orbital-fleet-map.png'));
    console.log('Captured 01-orbital-fleet-map.png');

    // 2. Agent inspector panel on claude:checkout-api@studio.
    await cdp.clickApp('#watch li[data-addr="claude:checkout-api@studio"]');
    await sleep(1600);
    await cdp.screenshot(path.join(OUT_DIR, '02-agent-inspector-panel.png'));
    console.log('Captured 02-agent-inspector-panel.png');

    // 3. Cross-machine channel conversation view.
    await cdp.clickApp('#panel-close');
    await sleep(500);
    await cdp.clickApp('#feed-list li[data-action="channel"][data-a*="checkout-api"], #feed-list li[data-action="channel"][data-b*="checkout-api"]');
    await sleep(1600);
    await cdp.evalApp(`
      const det = doc.querySelector('#panel-body ul.convo li details');
      if (det) det.open = true;
    `);
    await sleep(300);
    await cdp.screenshot(path.join(OUT_DIR, '03-channel-conversation.png'));
    console.log('Captured 03-channel-conversation.png');

    // 4. Fleet table drawer.
    await cdp.evalApp(`
      doc.getElementById('panel-close').click();
      doc.getElementById('btn-table').click();
    `);
    await sleep(1400);
    await cdp.screenshot(path.join(OUT_DIR, '04-fleet-table-drawer.png'));
    console.log('Captured 04-fleet-table-drawer.png');

    // 5. Attention & health alerts view.
    await cdp.evalApp(`
      const sel = doc.getElementById('color-by');
      sel.value = 'health';
      sel.dispatchEvent(new Event('change'));
      doc.getElementById('tab-attention').click();
    `);
    await sleep(1200);
    await cdp.screenshot(path.join(OUT_DIR, '05-attention-health-alerts.png'));
    console.log('Captured 05-attention-health-alerts.png');

    // 6. Spend & cost analytics view.
    await cdp.evalApp(`
      const sel = doc.getElementById('color-by');
      sel.value = 'spend';
      sel.dispatchEvent(new Event('change'));
      doc.getElementById('tab-spend').click();
    `);
    await sleep(1200);
    await cdp.screenshot(path.join(OUT_DIR, '06-spend-cost-analytics.png'));
    console.log('Captured 06-spend-cost-analytics.png');

    // 7. Session transcript log viewer.
    await cdp.clickApp('#drawer-close');
    await cdp.evalApp(`
      const sel = doc.getElementById('color-by');
      sel.value = 'kind';
      sel.dispatchEvent(new Event('change'));
    `);
    await cdp.clickApp('#watch li[data-addr="codex:search-ranker@gpu-worker"]');
    await sleep(1600);
    await cdp.clickApp('button[data-action="open-log"]');
    await sleep(800);
    await cdp.evalApp(`
      const firstDetails = doc.querySelector('#log-body details');
      if (firstDetails) firstDetails.open = true;
    `);
    await sleep(300);
    await cdp.screenshot(path.join(OUT_DIR, '07-session-transcript-viewer.png'));
    console.log('Captured 07-session-transcript-viewer.png');

    // 8. Broadcast preview modal.
    await cdp.evalApp(`
      const closeLog = doc.querySelector('button[data-action="log-close"]');
      if (closeLog) closeLog.click();
      doc.getElementById('panel-close').click();
      doc.getElementById('btn-broadcast').click();
    `);
    await sleep(400);
    await cdp.evalApp(`
      const scope = doc.getElementById('bc-scope');
      scope.value = 'pool:Payments';
      scope.dispatchEvent(new Event('change'));
      doc.getElementById('bc-subject').value = 'Pre-release freeze';
      doc.getElementById('bc-body').value = 'Finish verification on your active branch and report status back to checkout-api@studio.';
      doc.getElementById('bc-preview').click();
    `);
    await sleep(500);
    await cdp.screenshot(path.join(OUT_DIR, '08-broadcast-preview-modal.png'));
    console.log('Captured 08-broadcast-preview-modal.png');

    // 9. Hierarchy grouping & expanded swarm workers.
    await cdp.evalApp(`
      doc.getElementById('bc-close').click();
      const expand = doc.getElementById('opt-expand');
      if (!expand.checked) expand.click();
      doc.getElementById('btn-group').click();
    `);
    await sleep(300);
    await cdp.evalApp(`
      const preset = doc.querySelector('#group-panel button[data-preset="pool,host,project"]');
      if (preset) preset.click();
    `);
    await sleep(1100);
    await cdp.screenshot(path.join(OUT_DIR, '09-hierarchy-grouping-swarms.png'));
    console.log('Captured 09-hierarchy-grouping-swarms.png');

    cdp.close();
  } finally {
    chrome.kill('SIGKILL');
    server.close();
    fs.rmSync(profileDir, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
