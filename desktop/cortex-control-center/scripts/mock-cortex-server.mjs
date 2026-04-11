import http from "node:http";
import { fileURLToPath } from "node:url";

const DEFAULT_TOKEN = process.env.EXPECT_SMOKE_TOKEN || "expect-smoke-token";

function isoMinutesAgo(minutes) {
  return new Date(Date.now() - minutes * 60_000).toISOString();
}

function isoDaysAgo(days) {
  return new Date(Date.now() - days * 86_400_000).toISOString();
}

function nowIso() {
  return new Date().toISOString();
}

function buildSavingsFixture() {
  const dailyValues = [
    { saved: 92_000, boots: 5, hitRatePct: 96 },
    { saved: 104_000, boots: 6, hitRatePct: 95 },
    { saved: 121_000, boots: 5, hitRatePct: 97 },
    { saved: 118_000, boots: 6, hitRatePct: 96 },
    { saved: 134_000, boots: 7, hitRatePct: 96 },
    { saved: 148_000, boots: 8, hitRatePct: 97 },
    { saved: 162_000, boots: 8, hitRatePct: 98 },
    { saved: 171_000, boots: 7, hitRatePct: 97 },
    { saved: 188_000, boots: 9, hitRatePct: 98 },
    { saved: 214_000, boots: 10, hitRatePct: 98 },
    { saved: 226_000, boots: 9, hitRatePct: 97 },
    { saved: 239_000, boots: 11, hitRatePct: 98 },
    { saved: 253_000, boots: 10, hitRatePct: 99 },
    { saved: 268_000, boots: 11, hitRatePct: 98 },
  ];

  let savedTotal = 0;
  let servedTotal = 0;
  let baselineTotal = 0;

  const daily = dailyValues.map((row, index) => {
    const baseline = 480_000 + index * 24_000;
    const served = baseline - row.saved;
    savedTotal += row.saved;
    servedTotal += served;
    baselineTotal += baseline;
    return {
      date: isoDaysAgo(dailyValues.length - 1 - index).slice(0, 10),
      saved: row.saved,
      boots: row.boots,
      baseline,
      served,
      hitRatePct: row.hitRatePct,
    };
  });

  const totalBoots = daily.reduce((sum, row) => sum + row.boots, 0);

  return {
    summary: {
      totalSaved: savedTotal,
      totalServed: servedTotal,
      totalBaseline: baselineTotal,
      totalBoots,
      avgPercent: Math.round((savedTotal / baselineTotal) * 100),
      avgSavedPerBoot: Math.round(savedTotal / totalBoots),
      avgServedPerBoot: Math.round(servedTotal / totalBoots),
      avgBaselinePerBoot: Math.round(baselineTotal / totalBoots),
    },
    daily,
    cumulative: daily.map((row, index) => ({
      date: row.date,
      savedTotal: daily.slice(0, index + 1).reduce((sum, item) => sum + item.saved, 0),
    })),
    recallTrend: daily.map((row) => ({
      date: row.date,
      hitRatePct: row.hitRatePct,
    })),
    activityHeatmap: [
      { day: "Mon", hour: 9, count: 4 },
      { day: "Mon", hour: 10, count: 6 },
      { day: "Tue", hour: 14, count: 7 },
      { day: "Wed", hour: 15, count: 8 },
      { day: "Thu", hour: 20, count: 5 },
      { day: "Fri", hour: 11, count: 6 },
      { day: "Sat", hour: 13, count: 3 },
    ],
    byAgent: [
      { agent: "Codex", saved: 718_000, served: 1_140_000, percent: 39, boots: 18 },
      { agent: "Claude", saved: 904_000, served: 1_420_000, percent: 39, boots: 22 },
      { agent: "Factory Droid", saved: 532_000, served: 960_000, percent: 36, boots: 12 },
    ],
    byOperation: [
      { operation: "boot", saved: 1_430_000, served: 2_180_000, baseline: 3_610_000, events: 52 },
      { operation: "recall", saved: 541_000, served: 884_000, baseline: 1_425_000, events: 94 },
      { operation: "store", saved: 197_000, served: 410_000, baseline: 607_000, events: 40 },
      { operation: "tool", saved: 121_000, served: 215_000, baseline: 336_000, events: 27 },
    ],
    recent: [
      {
        timestamp: isoMinutesAgo(18),
        agent: "Codex",
        percent: 42,
        served: 36_000,
        baseline: 62_000,
        saved: 26_000,
        admitted: 12,
        rejected: 3,
      },
      {
        timestamp: isoMinutesAgo(62),
        agent: "Claude",
        percent: 38,
        served: 41_000,
        baseline: 66_000,
        saved: 25_000,
        admitted: 14,
        rejected: 2,
      },
      {
        timestamp: isoMinutesAgo(135),
        agent: "Factory Droid",
        percent: 35,
        served: 34_000,
        baseline: 52_000,
        saved: 18_000,
        admitted: 9,
        rejected: 1,
      },
    ],
  };
}

function buildFixture() {
  const savings = buildSavingsFixture();
  return {
    health: {
      status: "ok",
      embedding_status: "available",
      storage_bytes: 12_914_688,
      backup_count: 3,
      log_bytes: 412_672,
      stats: {
        memories: 239,
        decisions: 61,
        events: 812,
      },
    },
    sessions: {
      sessions: [
        {
          sessionId: "codex-smoke",
          agent: "Codex",
          description: "Validating desktop flows",
          project: "cortex-control-center",
          files: ["src/App.jsx", "src/BrainVisualizer.jsx", ".github/workflows/ci.yml"],
          lastHeartbeat: isoMinutesAgo(2),
        },
        {
          sessionId: "claude-review",
          agent: "Claude",
          description: "Reviewing analytics polish",
          project: "desktop polish",
          files: ["src/styles.css"],
          lastHeartbeat: isoMinutesAgo(6),
        },
      ],
    },
    locks: {
      locks: [
        {
          id: "lock-1",
          path: "desktop/cortex-control-center/src/App.jsx",
          agent: "Codex",
          expiresAt: new Date(Date.now() + 52 * 60_000).toISOString(),
        },
      ],
    },
    tasks: {
      tasks: [
        {
          taskId: "task-0",
          title: "Wire operator actions into the Work surface",
          description: "Claim, complete, abandon, message, unlock, and ack should all work from one operator control.",
          status: "pending",
          priority: "high",
          project: "desktop",
          files: ["src/App.jsx", "src/live-surface.js"],
        },
        {
          taskId: "task-1",
          title: "Smoke verify analytics shell",
          status: "in_progress",
          priority: "high",
          claimedBy: "Codex",
          summary: "Checking auth bootstrap and expect harness",
          claimedAt: isoMinutesAgo(11),
          project: "desktop",
        },
        {
          taskId: "task-2",
          title: "Review brain HUD spacing",
          status: "done",
          priority: "medium",
          claimedBy: "Claude",
          summary: "CSS overlap pass complete",
          completedAt: isoMinutesAgo(37),
          project: "desktop",
        },
      ],
    },
    feed: {
      entries: [
        {
          id: "feed-1",
          kind: "status",
          agent: "Codex",
          timestamp: isoMinutesAgo(9),
          priority: "high",
          tokens: 318,
          summary: "Expect smoke harness switched to mock-backed auth bootstrap.",
          files: ["scripts/run-expect-smoke.mjs", "scripts/mock-cortex-server.mjs"],
        },
        {
          id: "feed-2",
          kind: "review",
          agent: "Claude",
          timestamp: isoMinutesAgo(31),
          summary: "Brain view title updated to Neural topology.",
          files: ["src/BrainVisualizer.jsx"],
        },
        {
          id: "feed-3",
          kind: "task_complete",
          agent: "Factory Droid",
          timestamp: isoMinutesAgo(2),
          summary: "Completed live surface review pass for the task queue.",
          taskId: "task-9",
          files: ["src/live-surface.test.js"],
        },
      ],
    },
    messages: {
      messages: [
        {
          id: "msg-1",
          from: "Claude",
          to: "Codex",
          timestamp: isoMinutesAgo(22),
          message: "Analytics summary box looks stable in the latest build.",
        },
        {
          id: "msg-2",
          from: "Factory Droid",
          to: "Codex",
          timestamp: isoMinutesAgo(48),
          message: "CI notes ready once the smoke harness lands.",
        },
      ],
    },
    activity: {
      activities: [
        {
          id: "activity-1",
          agent: "Codex",
          timestamp: isoMinutesAgo(5),
          description: "Patched CI workflow and local expect smoke scripts.",
          files: [".github/workflows/ci.yml", "desktop/cortex-control-center/package.json"],
        },
        {
          id: "activity-2",
          agent: "Claude",
          timestamp: isoMinutesAgo(28),
          description: "Reviewed brain HUD overlap fix and analytics spacing.",
          files: ["src/styles.css", "src/BrainVisualizer.jsx"],
        },
      ],
    },
    savings,
    conflicts: {
      pairs: [
        {
          left: {
            id: 41,
            source_agent: "Codex",
            created_at: isoMinutesAgo(240),
            decision: "Run browser verification from a mocked Cortex API.",
            context: "Keeps expect smoke deterministic for source builds and CI.",
            confidence: 0.93,
          },
          right: {
            id: 42,
            source_agent: "Claude",
            created_at: isoMinutesAgo(238),
            decision: "Drive browser verification against a live daemon only.",
            context: "Rejected because auth and local state make it flaky.",
            confidence: 0.42,
          },
        },
      ],
    },
    dump: {
      memories: [
        { id: 1, source: "memory::expect_smoke_ci", text: "Expect smoke should use a mock daemon on port 7437 for browser fallback.", source_agent: "Codex", score: 5 },
        { id: 2, source: "memory::analytics_projection", text: "Monte Carlo chart now shows a fixed summary box instead of stacked endpoint labels.", source_agent: "Codex", score: 4 },
        { id: 3, source: "memory::brain_title", text: "Brain page headline should use Neural topology, not Jarvis.", source_agent: "Claude", score: 4 },
        { id: 4, source: "memory::activity_stream", text: "Recent activity panel should be populated for smoke coverage.", source_agent: "Factory Droid", score: 3 },
      ],
      decisions: [
        { id: 11, decision: "Prefer repo-local expect-cli via npm exec for deterministic versions.", context: "Matches npm ci installs in local dev and CI.", source_agent: "Codex", score: 5, status: "active" },
        { id: 12, decision: "Use browser bootstrap query params for smoke auth.", context: "Avoids cookie extraction and live daemon token reads.", source_agent: "Codex", score: 4, status: "active" },
        { id: 13, decision: "Keep CI smoke opt-in until an agent provider secret is configured.", context: "Hosted runners do not come with agent auth by default.", source_agent: "Claude", score: 4, status: "active", disputes_id: 14 },
        { id: 14, decision: "Require live daemon auth for browser smoke.", context: "Superseded by deterministic mock-backed auth.", source_agent: "Factory Droid", score: 1, status: "disputed" },
      ],
    },
    peek: {
      matches: [
        { source: "memory::expect_smoke_ci", relevance: 0.98, method: "keyword" },
        { source: "memory::analytics_projection", relevance: 0.93, method: "semantic" },
        { source: "decision::ci_opt_in", relevance: 0.89, method: "keyword" },
      ],
    },
    recall: {
      results: [
        {
          source: "memory::expect_smoke_ci",
          excerpt: "Expect smoke should use a mock daemon on port 7437 so browser fallback auth is deterministic in CI and source builds.",
          relevance: 0.98,
          method: "keyword",
        },
        {
          source: "memory::analytics_projection",
          excerpt: "Monte Carlo projection labels were moved into a summary box to avoid overlap with the endpoint marker.",
          relevance: 0.93,
          method: "semantic",
        },
      ],
    },
    feedAcks: new Map(),
  };
}

async function readJsonBody(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }
  if (!chunks.length) return {};

  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    return {};
  }
}

function buildFeedResponse(fixture, url) {
  let entries = [...fixture.feed.entries].sort(
    (left, right) => new Date(left.timestamp).getTime() - new Date(right.timestamp).getTime(),
  );

  const kind = url.searchParams.get("kind");
  if (kind && kind !== "all") {
    entries = entries.filter((entry) => entry.kind === kind);
  }

  const unreadOnly = url.searchParams.get("unread") === "true";
  const agent = (url.searchParams.get("agent") || "").trim();
  if (unreadOnly && agent) {
    const lastSeenId = fixture.feedAcks.get(agent);
    if (lastSeenId) {
      const ackIndex = entries.findIndex((entry) => entry.id === lastSeenId);
      if (ackIndex >= 0) {
        entries = entries.slice(ackIndex + 1);
      }
    }
    entries = entries.filter((entry) => entry.agent !== agent);
  }

  return { entries };
}

function buildMessagesResponse(fixture, url) {
  const agent = (url.searchParams.get("agent") || "").trim();
  const messages = agent
    ? fixture.messages.messages.filter((entry) => entry.to === agent)
    : fixture.messages.messages;

  return { messages };
}

function sendJson(response, statusCode, body) {
  response.writeHead(statusCode, {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Headers": "Authorization, Content-Type, X-Cortex-Request",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Cache-Control": "no-store",
    "Content-Type": "application/json; charset=utf-8",
  });
  response.end(JSON.stringify(body));
}

function isAuthorized(request, token) {
  return request.headers.authorization === `Bearer ${token}`;
}

export async function startMockCortexServer({ host = "127.0.0.1", port = 7437, token = DEFAULT_TOKEN } = {}) {
  const fixture = buildFixture();
  const protectedPrefixes = [
    "/sessions",
    "/locks",
    "/tasks",
    "/feed",
    "/messages",
    "/activity",
    "/savings",
    "/conflicts",
    "/dump",
    "/peek",
    "/recall",
    "/resolve",
  ];

  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url || "/", `http://${request.headers.host || `${host}:${port}`}`);

    if (request.method === "OPTIONS") {
      response.writeHead(204, {
        "Access-Control-Allow-Origin": "*",
        "Access-Control-Allow-Headers": "Authorization, Content-Type, X-Cortex-Request",
        "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
      });
      response.end();
      return;
    }

    if (protectedPrefixes.some((prefix) => url.pathname.startsWith(prefix)) && !isAuthorized(request, token)) {
      sendJson(response, 401, { error: "Unauthorized" });
      return;
    }

    if (request.method === "GET" && url.pathname === "/health") {
      sendJson(response, 200, fixture.health);
      return;
    }
    if (request.method === "GET" && url.pathname === "/events/stream") {
      response.writeHead(200, {
        "Access-Control-Allow-Origin": "*",
        "Cache-Control": "no-store",
        Connection: "keep-alive",
        "Content-Type": "text/event-stream; charset=utf-8",
      });
      response.write("event: connected\n");
      response.write('data: {"source":"expect-smoke"}\n\n');
      const heartbeat = setInterval(() => {
        response.write("event: feed\n");
        response.write("data: {}\n\n");
      }, 20_000);
      request.on("close", () => {
        clearInterval(heartbeat);
      });
      return;
    }
    if (request.method === "GET" && url.pathname === "/sessions") {
      sendJson(response, 200, fixture.sessions);
      return;
    }
    if (request.method === "GET" && url.pathname === "/locks") {
      sendJson(response, 200, fixture.locks);
      return;
    }
    if (request.method === "GET" && url.pathname === "/tasks") {
      sendJson(response, 200, fixture.tasks);
      return;
    }
    if (request.method === "GET" && url.pathname === "/feed") {
      sendJson(response, 200, buildFeedResponse(fixture, url));
      return;
    }
    if (request.method === "GET" && url.pathname === "/messages") {
      sendJson(response, 200, buildMessagesResponse(fixture, url));
      return;
    }
    if (request.method === "GET" && url.pathname === "/activity") {
      sendJson(response, 200, fixture.activity);
      return;
    }
    if (request.method === "GET" && url.pathname === "/savings") {
      sendJson(response, 200, fixture.savings);
      return;
    }
    if (request.method === "GET" && url.pathname === "/conflicts") {
      sendJson(response, 200, fixture.conflicts);
      return;
    }
    if (request.method === "GET" && url.pathname === "/dump") {
      sendJson(response, 200, fixture.dump);
      return;
    }
    if (request.method === "GET" && url.pathname === "/peek") {
      sendJson(response, 200, fixture.peek);
      return;
    }
    if (request.method === "GET" && url.pathname === "/recall") {
      sendJson(response, 200, fixture.recall);
      return;
    }
    if (request.method === "POST" && url.pathname === "/resolve") {
      sendJson(response, 200, { ok: true, action: "resolved" });
      return;
    }
    if (request.method === "POST" && url.pathname === "/tasks/claim") {
      const body = await readJsonBody(request);
      const task = fixture.tasks.tasks.find((entry) => entry.taskId === body.taskId);
      if (!task) {
        sendJson(response, 404, { error: "task_not_found" });
        return;
      }
      if (task.status === "claimed") {
        sendJson(response, 409, { error: "task_already_claimed", claimedBy: task.claimedBy });
        return;
      }
      if (task.status === "completed" || task.status === "done") {
        sendJson(response, 409, { error: "task_already_completed" });
        return;
      }
      task.status = "claimed";
      task.claimedBy = body.agent;
      task.claimedAt = nowIso();
      sendJson(response, 200, { claimed: true, taskId: task.taskId });
      return;
    }
    if (request.method === "POST" && url.pathname === "/tasks/complete") {
      const body = await readJsonBody(request);
      const task = fixture.tasks.tasks.find((entry) => entry.taskId === body.taskId);
      if (!task) {
        sendJson(response, 404, { error: "task_not_found" });
        return;
      }
      if (task.claimedBy !== body.agent) {
        sendJson(response, 403, { error: "not_task_holder", claimedBy: task.claimedBy || null });
        return;
      }
      task.status = "completed";
      task.completedAt = nowIso();
      task.summary = body.summary || task.summary || "";
      fixture.feed.entries.push({
        id: `feed-${fixture.feed.entries.length + 1}`,
        kind: "task_complete",
        agent: body.agent,
        timestamp: task.completedAt,
        summary: `Completed: ${task.title}`,
        taskId: task.taskId,
        files: Array.isArray(task.files) ? task.files : [],
      });
      sendJson(response, 200, { completed: true, taskId: task.taskId });
      return;
    }
    if (request.method === "POST" && url.pathname === "/tasks/abandon") {
      const body = await readJsonBody(request);
      const task = fixture.tasks.tasks.find((entry) => entry.taskId === body.taskId);
      if (!task) {
        sendJson(response, 404, { error: "task_not_found" });
        return;
      }
      if (task.claimedBy !== body.agent) {
        sendJson(response, 403, { error: "not_task_holder", claimedBy: task.claimedBy || null });
        return;
      }
      task.status = "pending";
      delete task.claimedBy;
      delete task.claimedAt;
      sendJson(response, 200, { abandoned: true, taskId: task.taskId, status: "pending" });
      return;
    }
    if (request.method === "POST" && url.pathname === "/tasks/delete") {
      const body = await readJsonBody(request);
      fixture.tasks.tasks = fixture.tasks.tasks.filter((entry) => entry.taskId !== body.taskId);
      sendJson(response, 200, { ok: true, deleted: true, taskId: body.taskId || null });
      return;
    }
    if (request.method === "POST" && url.pathname === "/message") {
      const body = await readJsonBody(request);
      fixture.messages.messages.push({
        id: `msg-${fixture.messages.messages.length + 1}`,
        from: body.from,
        to: body.to,
        timestamp: nowIso(),
        message: body.message,
      });
      sendJson(response, 200, { sent: true, messageId: `msg-${fixture.messages.messages.length}` });
      return;
    }
    if (request.method === "POST" && url.pathname === "/unlock") {
      const body = await readJsonBody(request);
      fixture.locks.locks = fixture.locks.locks.filter(
        (entry) => !(entry.path === body.path && entry.agent === body.agent),
      );
      sendJson(response, 200, { unlocked: true });
      return;
    }
    if (request.method === "POST" && url.pathname === "/feed/ack") {
      const body = await readJsonBody(request);
      fixture.feedAcks.set(body.agent, body.lastSeenId);
      sendJson(response, 200, { acked: true });
      return;
    }

    sendJson(response, 404, { error: `No mock handler for ${request.method} ${url.pathname}` });
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, resolve);
  });

  return {
    token,
    baseUrl: `http://${host}:${port}`,
    close: () =>
      new Promise((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()));
      }),
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const host = process.env.EXPECT_SMOKE_HOST || "127.0.0.1";
  const port = Number(process.env.EXPECT_SMOKE_API_PORT || "7438");
  const server = await startMockCortexServer({ host, port, token: DEFAULT_TOKEN });
  console.log(`[expect-smoke] mock Cortex listening at ${server.baseUrl}`);
}
