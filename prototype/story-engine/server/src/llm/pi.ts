/**
 * PiLlm — story-engine LLM adapter over the pi SDK.
 *
 * Uses `@earendil-works/pi-ai` + `@earendil-works/pi-coding-agent` as the
 * client, driving the same provider and model pi uses in this shell:
 *
 *   provider  makora          https://inference.makora.com/v1
 *   model     deepseek-ai/DeepSeek-V4-Flash
 *   key       from ~/.pi/agent/auth.json ("makora" entry)
 *
 * The raw pi-ai `streamSimple` path currently throws on this endpoint
 * ("block.name.length"), so we use the supported AgentSession path, which
 * handles streaming, auth, and context. See README "LLM configuration".
 *
 * Design notes:
 * - Each conversation owns one DefaultResourceLoader + AgentSession. The
 *   loader carries the character prompt via `systemPromptOverride` because
 *   the session otherwise resets it to a coding-agent base prompt each turn.
 * - cwd is a neutral temp dir so DefaultResourceLoader does not inject the
 *   repository AGENTS.md into the NPC context.
 * - Tools are disabled; the NPC is a plain dialogue model, not an agent.
 */
import {
  createAgentSession,
  DefaultResourceLoader,
  ModelRuntime,
  SessionManager,
  type AgentSession,
} from "@earendil-works/pi-coding-agent";
import { tmpdir } from "node:os";
import { join } from "node:path";

export interface PiChat {
  /** Send one player line; stream decoded tokens to `onToken`, return full reply. */
  send(content: string, onToken?: (delta: string) => void): Promise<string>;
  /** Release the session and extension resources. */
  close(): void;
}

const NEUTRAL_CWD = join(tmpdir(), "story-engine");

class SessionChat implements PiChat {
  private readonly session: AgentSession;
  private readonly subscribers = new Set<(delta: string) => void>();
  private readonly unsubscribe: () => void;
  private closed = false;

  constructor(session: AgentSession) {
    this.session = session;
    this.unsubscribe = session.subscribe((event) => {
      if (event.type === "message_update" && event.assistantMessageEvent?.type === "text_delta") {
        const delta = event.assistantMessageEvent.delta;
        for (const sink of this.subscribers) sink(delta);
      }
    });
  }

  async send(content: string, onToken?: (delta: string) => void): Promise<string> {
    let acc = "";
    const sink = (delta: string) => {
      acc += delta;
      onToken?.(delta);
    };
    this.subscribers.add(sink);
    try {
      await this.session.prompt(content);
    } finally {
      this.subscribers.delete(sink);
    }
    return acc;
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.unsubscribe();
    this.session.dispose();
  }
}

export class PiLlm {
  private readonly modelRuntime: ModelRuntime;
  private readonly agentDir: string;
  private readonly modelId: string;
  private static instance: PiLlm | null = null;

  private constructor(modelRuntime: ModelRuntime, agentDir: string, modelId: string) {
    this.modelRuntime = modelRuntime;
    this.agentDir = agentDir;
    this.modelId = modelId;
  }

  /** Create the single shared runtime. Read auth from pi's own store. */
  static async init(opts: { agentDir?: string; modelId?: string } = {}): Promise<PiLlm> {
    if (PiLlm.instance) return PiLlm.instance;
    const agentDir = opts.agentDir ?? join(process.env.HOME ?? "", ".pi", "agent");
    const modelId = opts.modelId ?? "deepseek-ai/DeepSeek-V4-Flash";
    const modelRuntime = await ModelRuntime.create();
    PiLlm.instance = new PiLlm(modelRuntime, agentDir, modelId);
    return PiLlm.instance;
  }

  /**
   * Open a new conversation session for one NPC with a fixed character prompt.
   * The makora provider registers when the loader binds the makora extension
   * (discovered from agentDir), so the model resolves after session creation.
   */
  async newConversation(characterPrompt: string): Promise<SessionChat> {
    const loader = new DefaultResourceLoader({
      cwd: NEUTRAL_CWD,
      agentDir: this.agentDir,
      systemPromptOverride: () => characterPrompt,
    });
    await loader.reload();

    const { session } = await createAgentSession({
      cwd: NEUTRAL_CWD,
      agentDir: this.agentDir,
      modelRuntime: this.modelRuntime,
      resourceLoader: loader,
      noTools: "all",
      thinkingLevel: "off",
      sessionManager: SessionManager.inMemory(),
    });

    const model = this.modelRuntime.getModel("makora", this.modelId) ?? undefined;
    if (!model) {
      session.dispose();
      throw new Error(`makora model "${this.modelId}" not registered`);
    }
    await session.setModel(model);

    return new SessionChat(session);
  }
}
