// agent-relay for pi: registers the session, tells the model its relay address,
// and delivers relayed messages, starting a turn when the session is idle.
import { spawn } from "node:child_process"
import { readdirSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

const RELAY = fileURLToPath(new URL("../scripts/relay.py", import.meta.url))
const POLL_MS = 2000

function hook(event: string, payload: Record<string, unknown>): Promise<any> {
  return new Promise((resolve) => {
    let out = ""
    let child: any
    try {
      child = spawn("python3", [RELAY, "hook", "--agent", "pi", event], { stdio: ["pipe", "pipe", "ignore"] })
    } catch {
      return resolve({})
    }
    const timer = setTimeout(() => child.kill(), 15000)
    child.stdout.on("data", (chunk: Buffer) => (out += chunk))
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

export default function (pi: any) {
  let sid: string | undefined
  let base: Record<string, unknown> = {}
  let lastCtx: any
  let timer: ReturnType<typeof setInterval> | undefined
  let polling = false

  const inbox = () => join(process.env.AGENT_RELAY_HOME || join(homedir(), ".agent-relay"), "inbox", sid || "-")

  function hasMail(): boolean {
    try {
      return readdirSync(inbox()).some((name) => name.endsWith(".json"))
    } catch {
      return false
    }
  }

  async function poll() {
    if (polling || !sid || !hasMail()) return
    polling = true
    try {
      const reply = await hook("inbox", base)
      if (reply.context) {
        pi.sendMessage(
          { customType: "agent-relay-message", content: reply.context, display: true },
          { triggerTurn: true, deliverAs: "followUp" },
        )
      }
    } finally {
      polling = false
    }
  }

  pi.on("session_start", async (_event: any, ctx: any) => {
    lastCtx = ctx
    const sm = ctx.sessionManager
    sid = String(sm.getSessionId())
    base = { session_id: sid, cwd: sm.getCwd?.() ?? process.cwd(), transcript_path: sm.getSessionFile?.(), pid: process.pid }
    // Bash tool calls inherit this, so `relay send` knows who is sending.
    process.env.AGENT_RELAY_PI_SESSION = sid
    const reply = await hook("session-start", base)
    if (reply.context) {
      const seen = (sm.getEntries?.() ?? []).some(
        (e: any) => e.customType === "agent-relay" && JSON.stringify(e).includes(String(reply.address)),
      )
      if (!seen) pi.sendMessage({ customType: "agent-relay", content: reply.context, display: false }, { deliverAs: "nextTurn" })
    }
    if (timer) clearInterval(timer)
    timer = setInterval(() => void poll(), POLL_MS)
    timer.unref?.() // never keep a print-mode run alive
  })

  pi.on("agent_start", async (_event: any, ctx: any) => {
    lastCtx = ctx
    if (sid) await hook("prompt", base)
  })

  pi.on("agent_settled", async (_event: any, ctx: any) => {
    lastCtx = ctx
    if (sid) await hook("stop", base)
  })

  pi.on("session_shutdown", async () => {
    if (timer) clearInterval(timer)
    timer = undefined
    if (sid) await hook("stop", base)
    void lastCtx
  })
}
