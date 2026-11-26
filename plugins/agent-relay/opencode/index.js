// agent-relay for opencode: tells each top-level session its relay address.
// Messages reach opencode sessions through the opencode service (`opencode api
// session.prompt`), so this plugin only has to announce the address and CLI.
import { spawn } from "node:child_process"
import { fileURLToPath } from "node:url"

const RELAY = fileURLToPath(new URL("../scripts/relay.py", import.meta.url))
const KEY = "agent-relay"

function hook(event, payload) {
  return new Promise((resolve) => {
    let out = ""
    let child
    try {
      child = spawn("python3", [RELAY, "hook", "--agent", "opencode", event], {
        stdio: ["pipe", "pipe", "ignore"],
      })
    } catch {
      return resolve({})
    }
    const timer = setTimeout(() => child.kill(), 15000)
    child.stdout.on("data", (chunk) => (out += chunk))
    child.on("error", () => resolve({}))
    child.on("close", () => {
      clearTimeout(timer)
      try {
        resolve(JSON.parse(out.trim().split("\n").pop() || "{}"))
      } catch {
        resolve({})
      }
    })
    child.stdin.end(JSON.stringify(payload))
  })
}

function rows(page) {
  return (Array.isArray(page) && page) || page?.data || page?.items || []
}

export default {
  id: "agent-relay",
  async setup(ctx) {
    const done = new Set()

    async function announce(sessionID, directory) {
      if (!sessionID || done.has(sessionID)) return
      done.add(sessionID)
      const reply = await hook("session-start", { session_id: sessionID, cwd: directory, pid: process.pid })
      if (!reply.context) return
      try {
        const entries = rows(await ctx.session.instructions.entry.list({ sessionID }))
        if (entries.some((e) => e.key === KEY && e.value === reply.context)) return
        await ctx.session.instructions.entry.put({ sessionID, key: KEY, value: reply.context })
      } catch (error) {
        done.delete(sessionID)
        console.error(`agent-relay: could not add the relay address to ${sessionID}: ${error}`)
      }
    }

    const controller = new AbortController()
    void (async () => {
      for await (const event of ctx.event.subscribe({ types: ["session.created"], signal: controller.signal })) {
        const data = event.data || {}
        if (!data.parentID) await announce(data.sessionID, data.location?.directory)
      }
    })().catch(() => {})

    // Sessions that were already open when the plugin loaded.
    void (async () => {
      const page = await ctx.session.list({ limit: 20 })
      const day = Date.now() - 24 * 3600 * 1000
      for (const s of rows(page)) {
        if (s.parentID || s.time?.archived || (s.time?.updated ?? 0) < day) continue
        await announce(s.id, s.location?.directory)
      }
    })().catch(() => {})

    return () => controller.abort()
  },
}
