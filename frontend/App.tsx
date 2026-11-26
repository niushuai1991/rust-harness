import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  ArrowDown,
  ArrowUp,
  Bot,
  Check,
  ChevronDown,
  Copy,
  Download,
  Folder,
  FolderOpen,
  GitCompareArrows,
  Goal,
  ListTodo,
  MessageSquare,
  Paperclip,
  Pencil,
  Pin,
  Plus,
  Quote,
  Search,
  Send,
  ShieldCheck,
  Square,
  Target,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { createPortal } from "react-dom";
import { command, desktop, errorText, resume, send } from "./api";
import { BackgroundTasksPanel } from "./BackgroundTasksPanel";
import { ComposerInput } from "./ComposerInput";
import { ComposerJobsPanel } from "./ComposerJobsPanel";
import {
  ConversationEvents,
  Markdown,
  PLAN_STATE_KINDS,
  noteLiveStateEvent,
} from "./ConversationEvents";
import { Modal } from "./Modal";
import ModelSelect from "./ModelSelect";
import { PluginThemeHost } from "./PluginContributionsPanel";
import { PluginPanel } from "./PluginPanel";
import { PresetsPanel } from "./PresetsPanel";
import {
  NO_PROJECT_NAME,
  filterByTitle,
  formatFullTime,
  relativeLabel,
  subagentsOf,
  upsertSession,
  withoutSubagents,
  type ViewMode,
} from "./sessionList";
import { SessionSidebar } from "./SessionSidebar";
import {
  filterSkillRows,
  skillRowsFrom,
  skillTokenUnderCursor,
  type SkillRow,
  type SkillToken,
} from "./skills";
import { SkillsPanel } from "./SkillsPanel";
import {
  GOAL_PROMPT,
  INIT_PROMPT,
  SLASH_COMMANDS,
  commandTokenUnderCursor,
  expandSlashPrompt,
  filterSlashCommands,
  parseSideSlashCommand,
  type SlashSpec,
} from "./slashCommands";
import { useChat } from "./store";
import { SubagentPanel } from "./SubagentPanel";
import { TurnNavigator } from "./TurnNavigator";
import { createSessionLoadLock } from "./turnNavigatorText";
import { KNOWN_APPROVALS } from "./types";
import { UsageRing } from "./UsageRing";
import { useTurnNavigation } from "./useTurnNavigation";
export { ConversationEvents, EventCard } from "./ConversationEvents";
export { Modal } from "./Modal";

import { ApprovalPrompt } from "./ApprovalPrompt";
import { AutomationsPanel } from "./AutomationsPanel";
import { QuestionCard } from "./QuestionCard";
import { SettingsPage, type SettingsCategoryId } from "./SettingsPage";
import { SidePanel } from "./SidePanel";
import { useSidePanel } from "./sidePanelStore";
import type {
  ApprovalPolicy,
  ApprovalRequest,
  ArtifactMetadata,
  AskAnswer,
  AskRequest,
  DefaultSelection,
  ExecutionMode,
  LiveEvent,
  ModelConfig,
  ProviderInfo,
  SessionEvent,
  SessionInfo,
  SkillInfo,
  TimelinePage,
  WorkspaceInfo,
  WorkspaceMergeSummary,
  WorkspaceRemovalSummary,
  WorkspaceRevealSummary,
} from "./types";

type Panel = "presets" | "skills" | "plugins" | "tasks" | "automations" | null;
type SessionContextMenu = { sessionId: string; x: number; y: number };
type MessageContextMenu = { x: number; y: number; text: string };
type SessionRenameDraft = { id: string; name: string; dialogId: number };
const TERMINAL_STATUS: { [status: string]: string } = {
  running: "运行中",
  exited: "已退出",
  terminated: "已终止",
  interrupted: "已中断",
};
// 桌面权限模式（与 contracts 五档一一对应）；改动仍只落隔离工作副本，进原项目仍需 apply 确认。
const APPROVAL_MODES: { value: ApprovalPolicy; label: string; description: string }[] = [
  { value: "ask", label: "每次确认", description: "每个工具调用前逐一确认" },
  { value: "edit", label: "自动编辑", description: "写文件自动放行，其余仍确认" },
  { value: "guarded", label: "自主", description: "自动执行，危险命令与交互终端仍需确认" },
  { value: "auto", label: "完全访问", description: "全部自动放行（改动仍只落工作副本）" },
  { value: "never", label: "只读", description: "需审批的工具一律拒绝" },
];

// 侧栏宽度：拖拽 200px ~ 40% 视口（上限 480px），localStorage['sidebar-width'] 记忆（与右侧面板同模式）。
const SIDEBAR_WIDTH_KEY = "sidebar-width";
const SIDEBAR_MIN_WIDTH = 200;
function savedSidebarWidth() {
  const value = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY));
  return Number.isFinite(value) && value >= SIDEBAR_MIN_WIDTH ? value : 264;
}
// /goal 用法提示已并入目标模式激活提示；目标文本展开用 GOAL_PROMPT（slashCommands.ts）。
const basename = (path: string) => path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
const dirOf = (path: string) => {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 1 ? parts.slice(0, -1).join("/") : "";
};
const printable = (value: unknown) =>
  typeof value === "string" ? value : JSON.stringify(value, null, 2);
type SessionPopState = {
  title: string;
  time: string;
  rect: { left: number; top: number; right: number };
};
/// 会话行悬停浮层：完整会话名 + 更新时间。fixed 定位脱离侧栏滚动容器避免被裁剪；
/// 渲染后按实际尺寸校正到视口内（右侧放不下换到行左侧），纯展示不拦截鼠标。
function SessionPopCard({
  title,
  time,
  rect,
}: {
  title: string;
  time: string;
  rect: SessionPopState["rect"];
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    let left = rect.right + 10;
    if (left + el.offsetWidth > window.innerWidth - 8)
      left = Math.max(8, rect.left - el.offsetWidth - 10);
    setPos({
      left,
      top: Math.min(
        Math.max(8, rect.top - 6),
        Math.max(8, window.innerHeight - el.offsetHeight - 8),
      ),
    });
  }, [rect]);
  return (
    <div
      ref={ref}
      className="session-pop"
      role="tooltip"
      style={pos ? { left: pos.left, top: pos.top } : undefined}
    >
      <div className="pop-title">{title}</div>
      {time && <div className="pop-time">{time}</div>}
    </div>
  );
}
export default function App() {
  const [workspaces, setWorkspaces] = useState<WorkspaceInfo[]>([]),
    [workspaceId, setWorkspaceId] = useState("");
  // 项目目录是否已加载完成：workspace effect 需等它就绪才启动，避免初始空串占位 workspaceId 的加载与启动加载竞态。
  const [workspacesLoaded, setWorkspacesLoaded] = useState(false);
  // 创建态：正在等待 session_create 返回（草稿首条发送 / 组头新建）。只覆盖创建本身，
  // 不覆盖首轮执行（执行中由 running[s.id] 表达），避免新建聊天与输入框被整个回合锁死。
  const [sessions, setSessions] = useState<SessionInfo[]>([]),
    [sessionId, setSessionId] = useState(""),
    [sessionCreating, setSessionCreating] = useState(false);
  // 跨工作区全量会话（侧栏「最近 / 项目」视图数据源）；sessions 始终为当前工作区子集。
  const [allSessions, setAllSessions] = useState<SessionInfo[]>([]);
  // 已归档会话（设置页「存档管理」数据源）；打开设置时从后端刷新。
  const [archivedSessions, setArchivedSessions] = useState<SessionInfo[]>([]);
  const [models, setModels] = useState<ModelConfig[]>([]),
    [modelId, setModelId] = useState(""),
    [providers, setProviders] = useState<ProviderInfo[]>([]),
    [effort, setEffort] = useState<string | null>(null);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]),
    [panel, setPanel] = useState<Panel>(null),
    [settingsOpen, setSettingsOpen] = useState(false),
    [settingsSection, setSettingsSection] = useState<SettingsCategoryId | null>(null);
  // ask_user 提问：与审批同用轮询通道；答复后从列表移除。
  const [questions, setQuestions] = useState<AskRequest[]>([]);
  // 权限模式：默认档全局持久化；会话内切档记录显式覆盖（后端同口径：覆盖优先于默认档）。
  const [approvalMode, setApprovalMode] = useState<ApprovalPolicy>("ask"),
    [approvalOverrides, setApprovalOverrides] = useState<Record<string, ApprovalPolicy>>({});
  const [automationCount, setAutomationCount] = useState<number | null>(null);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const sideOpen = useSidePanel((s) => s.open),
    sideView = useSidePanel((s) => s.view),
    chatSessionId = useSidePanel((s) => s.chatSessionId);
  const [activeIds, setActiveIds] = useState<Set<string>>(() => new Set());
  const [text, setText] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [running, setRunning] = useState<Record<string, boolean>>({});
  const steerBusy = useRef(false);
  const steerRetry = useRef<{ key: string; id: string } | null>(null);
  const [cursor, setCursor] = useState(0),
    [slashIndex, setSlashIndex] = useState(0),
    [dismissedToken, setDismissedToken] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  // 补全弹层（/ 与 $）当前选中项：方向键移动选中项时滚动跟随用。
  const selectedOptionRef = useRef<HTMLButtonElement | null>(null);
  const [notice, setNotice] = useState("");
  // 滚动到底按钮：用户上滚离开底部后显示，点击一键回底并恢复自动跟随。
  const [showJumpBottom, setShowJumpBottom] = useState(false);
  const [planState, setPlanState] = useState<{
    id: string;
    enabled: boolean | null;
    busy: boolean;
  }>({ id: "", enabled: null, busy: false });
  const [draftPlan, setDraftPlan] = useState(false);
  // 草稿态执行位置覆盖：null 表示跟随项目默认；切换项目时清空覆盖。
  const [draftExecOverride, setDraftExecOverride] = useState<ExecutionMode | null>(null);
  // 计划抽屉联动：收到 todo/plan/goal 状态事件时递增，驱动打开中的「计划」视图重新拉取汇总。
  const [planPanelRevision, setPlanPanelRevision] = useState(0);
  const planRevision = useRef(0),
    planPending = useRef(new Set<string>());
  const exportNoticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (exportNoticeTimer.current !== null) clearTimeout(exportNoticeTimer.current);
    },
    [],
  );
  // 会话列表「最近 / 项目」视图：视图模式与项目折叠状态存 localStorage，跨重启保留。
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    try {
      return localStorage.getItem("sessionView") === "project" ? "project" : "recent";
    } catch {
      return "recent";
    }
  });
  const [collapsed, setCollapsed] = useState<string[]>(() => {
    try {
      const v = JSON.parse(localStorage.getItem("sessionCollapsed") ?? "[]");
      return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
    } catch {
      return [];
    }
  });
  const [query, setQuery] = useState("");
  const [sessionContextMenu, setSessionContextMenu] = useState<SessionContextMenu | null>(null),
    [sessionRename, setSessionRename] = useState<SessionRenameDraft | null>(null),
    [sessionRenameError, setSessionRenameError] = useState("");
  // 项目组头右键菜单与移除确认（移除只是隐藏入口，记录与会话原样保留）。
  const [wsContextMenu, setWsContextMenu] = useState<{
    workspaceId: string;
    x: number;
    y: number;
  } | null>(null);
  const [wsRemove, setWsRemove] = useState<{ id: string; name: string; running: boolean } | null>(
    null,
  );
  // 同源项目合并弹窗：sourcePath 圈定参与合并的同源项目，keepId 自动固定为
  // 创建时间最早的保留方（openMergeConfirm 内确定，用户不可改选）。
  const [wsMerge, setWsMerge] = useState<{ sourcePath: string } | null>(null),
    [wsMergeKeepId, setWsMergeKeepId] = useState("");
  const [sessionRenameSavingIds, setSessionRenameSavingIds] = useState<Set<string>>(
    () => new Set(),
  );
  // 会话行悬停浮层：行内去掉日期后，完整标题与时间移到 hover 卡片；延迟显示/关闭避免快速扫过闪烁。
  const [sessionPop, setSessionPop] = useState<SessionPopState | null>(null);
  const sessionPopTimers = useRef({ show: 0, hide: 0 });
  function openSessionPop(s: SessionInfo, node: HTMLButtonElement) {
    window.clearTimeout(sessionPopTimers.current.hide);
    window.clearTimeout(sessionPopTimers.current.show);
    sessionPopTimers.current.show = window.setTimeout(() => {
      const rect = node.getBoundingClientRect();
      setSessionPop({
        title: s.title,
        time: [formatFullTime(s.updatedAt), relativeLabel(s.updatedAt, new Date())]
          .filter(Boolean)
          .join(" · "),
        rect: { left: rect.left, top: rect.top, right: rect.right },
      });
    }, 350);
  }
  function scheduleSessionPopClose() {
    window.clearTimeout(sessionPopTimers.current.show);
    window.clearTimeout(sessionPopTimers.current.hide);
    sessionPopTimers.current.hide = window.setTimeout(() => setSessionPop(null), 120);
  }
  function dismissSessionPop() {
    window.clearTimeout(sessionPopTimers.current.show);
    window.clearTimeout(sessionPopTimers.current.hide);
    setSessionPop(null);
  }
  useEffect(() => {
    const onResize = () => dismissSessionPop();
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      window.clearTimeout(sessionPopTimers.current.show);
      window.clearTimeout(sessionPopTimers.current.hide);
    };
  }, []);
  const sessionRenameDialogId = useRef(0),
    sessionRenameSaving = useRef(new Set<string>());
  const [pluginRevision, setPluginRevision] = useState(0);
  const [attachments, setAttachments] = useState<ArtifactMetadata[]>([]),
    [importing, setImporting] = useState(false);
  // 消息引用：发送前暂存的选中文字，随消息发给模型（<quote> 标签），UI 上不可编辑只能删除。
  const [quotes, setQuotes] = useState<string[]>([]);
  const [msgContextMenu, setMsgContextMenu] = useState<MessageContextMenu | null>(null);
  const [plusOpen, setPlusOpen] = useState(false);
  // 输入框下方的子代理面板：承载从侧栏列表移走的子代理会话（当前工作区）。
  const [subagentOpen, setSubagentOpen] = useState(false),
    [jobsOpen, setJobsOpen] = useState(false),
    [goalPending, setGoalPending] = useState(false); // 目标模式：/goal 激活，下一条发送的消息展开为 GOAL_PROMPT 目标，发送或切换会话后退出
  // 新会话草稿态：点「新建聊天」不立即创建会话，composer 上方项目 chip 选定落点，发送首条消息时才真正 session_create。
  const [drafting, setDrafting] = useState(false),
    [draftWorkspaceId, setDraftWorkspaceId] = useState<string | null>(null);
  // 草稿文本按落点项目分桶（内存态，不落库）：key 为 `${draftWorkspaceId}`（null→'null'、''→无项目哨兵，互不碰撞）。
  // 切换会话/工作区/落点都不丢草稿；草稿被消费（首条发送成功）后清空对应桶。
  const [draftBuckets, setDraftBuckets] = useState<Record<string, string>>({});
  const [pjMenuOpen, setPjMenuOpen] = useState(false),
    [pjQuery, setPjQuery] = useState("");
  // 侧栏宽度：localStorage 记忆（sidebar-width），默认 264px；ref 供拖拽闭包取最新值。
  const [sidebarWidth, setSidebarWidth] = useState(savedSidebarWidth);
  const sidebarWidthRef = useRef(sidebarWidth);
  sidebarWidthRef.current = sidebarWidth;
  // 工作区编辑弹窗：create=打开工作区时命名（留空用目录名），edit=编辑项目名称与执行模式。
  const [wsDialog, setWsDialog] = useState<
    | { mode: "create"; path: string; name: string; execMode: ExecutionMode }
    | {
        mode: "edit";
        id: string;
        name: string;
        execMode: ExecutionMode;
        currentMode: ExecutionMode;
      }
    | null
  >(null);
  // 模型浮层卡片与切换标记（仅内存：切换模型在会话流里的临时提示，不落库）。
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [modelNotes, setModelNotes] = useState<ModelNote[]>([]);
  const modelNoteSeq = useRef(0);
  const sessionTitleOverrides = useRef(new Map<string, string>()),
    manualSessionTitles = useRef(new Set<string>());
  useEffect(() => {
    if (!plusOpen) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as Element | null)?.closest?.(".composer-plus-wrap")) setPlusOpen(false);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPlusOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [plusOpen]);
  useEffect(() => {
    if (!pjMenuOpen) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as Element | null)?.closest?.(".chip-row")) setPjMenuOpen(false);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPjMenuOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [pjMenuOpen]);
  useEffect(() => {
    if (!sessionContextMenu) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as Element | null)?.closest?.(".session-context-menu"))
        setSessionContextMenu(null);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSessionContextMenu(null);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [sessionContextMenu]);
  useEffect(() => {
    if (!msgContextMenu) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as Element | null)?.closest?.(".session-context-menu"))
        setMsgContextMenu(null);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMsgContextMenu(null);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [msgContextMenu]);
  useEffect(() => {
    if (!wsContextMenu) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as Element | null)?.closest?.(".session-context-menu")) setWsContextMenu(null);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setWsContextMenu(null);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [wsContextMenu]);
  const attachmentEpoch = useRef(0),
    activeSessionId = useRef(sessionId);
  activeSessionId.current = sessionId;
  const draftingRef = useRef(drafting);
  draftingRef.current = drafting; // 供异步 continuation 读取最新草稿态（effect 闭包里的 state 是过期快照）
  const selectionEpoch = useRef({ id: sessionId, workspace: workspaceId, version: 0 });
  if (selectionEpoch.current.id !== sessionId || selectionEpoch.current.workspace !== workspaceId)
    selectionEpoch.current = {
      id: sessionId,
      workspace: workspaceId,
      version: selectionEpoch.current.version + 1,
    };
  const titleGenerationRequests = useRef(new Set<string>());
  const pendingJump = useRef("");
  useEffect(() => {
    attachmentEpoch.current++;
    setAttachments([]);
    setImporting(false);
  }, [sessionId, workspaceId]);
  // 切换工作区时重置右侧面板（预览路径与视图归属工作区，避免串状态）。
  useEffect(() => {
    useSidePanel.getState().reset();
  }, [workspaceId]);
  // 模型在内嵌浏览器打开页面时自动弹出右侧「网页」视图。
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false,
      stop = () => {};
    listen<string>("web-panel-agent-open", (event) => {
      useSidePanel.getState().openWeb(String(event.payload));
    })
      .then((un) => {
        if (disposed) un();
        else stop = un;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      stop();
    };
  }, []);
  // 自动化任务到点执行（含手动触发）后同步侧栏：后端推事件，这里拉取单会话增量合入，不自动打开。
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false,
      stop = () => {};
    listen<string>("automation-session-created", (event) => {
      const id = String(event.payload);
      if (!id) return;
      command<SessionInfo>("session_get", { sessionId: id })
        .then((s) => {
          setAllSessions((old) => upsertSession(old, s));
          setSessions((old) =>
            old.some((item) => item.id === s.id)
              ? old
              : s.workspaceId === activeWorkspaceId.current
                ? [s, ...old]
                : old,
          );
        })
        .catch(() => {});
    })
      .then((un) => {
        if (disposed) un();
        else stop = un;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      stop();
    };
  }, []);
  // 侧栏「自动化」徽标：跟随工作区拉取任务数，面板增删后经 onChanged 刷新。
  useEffect(() => {
    if (!desktop()) {
      setAutomationCount(null);
      return;
    }
    let alive = true;
    command<unknown[]>("automation_list", { workspaceId })
      .then((list) => {
        if (alive) setAutomationCount(list.length);
      })
      .catch(() => {
        if (alive) setAutomationCount(null);
      });
    return () => {
      alive = false;
    };
  }, [workspaceId]);
  // 技能名单：跟随工作区拉取（composer $ 补全候选）；草稿态跟随草稿目标项目，读取失败退化为空（无补全）。
  const skillWorkspaceId = drafting ? (draftWorkspaceId ?? workspaceId) : workspaceId;
  useEffect(() => {
    if (!desktop()) {
      setSkills([]);
      return;
    }
    let alive = true;
    command<SkillInfo[]>("skill_list", { workspaceId: skillWorkspaceId })
      .then((list) => {
        if (alive) setSkills(Array.isArray(list) ? list : []);
      })
      .catch(() => {
        if (alive) setSkills([]);
      });
    return () => {
      alive = false;
    };
  }, [skillWorkspaceId]);
  const events = useChat((s) => s.events[sessionId] ?? EMPTY_EVENTS),
    timeline = useChat((s) => s.timelines[sessionId]),
    stream = useChat((s) => s.streams[sessionId]);
  // 主会话时间线固定：侧聊抽屉可能在同一工作区激活其他会话，避免主会话被 LRU 挤出后显示空白。
  useEffect(() => {
    if (!sessionId) return;
    const store = useChat.getState();
    store.pinTimeline(sessionId);
    return () => store.unpinTimeline(sessionId);
  }, [sessionId]);
  useEffect(() => {
    if (!desktop()) return;
    let disposed = false,
      stop: (() => void) | undefined;
    listen<SessionEvent>("session-steering-event", (event) => {
      useChat.getState().receive({ type: "durable", event: event.payload });
      if (event.payload.kind === "turn.end" && activeSessionId.current === event.payload.sessionId)
        void loadLatestTimeline(event.payload.sessionId);
    })
      .then((un) => {
        if (disposed) un();
        else stop = un;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
  const latestBoundary = [...events]
    .reverse()
    .find((event) => event.kind === "turn.start" || event.kind === "turn.end");
  const activeRunId =
    latestBoundary?.kind === "turn.start" && typeof latestBoundary.data.runId === "string"
      ? latestBoundary.data.runId
      : latestBoundary?.kind === "turn.end"
        ? null
        : ([...(timeline?.turns ?? [])].reverse().find((turn) => !turn.endSeq)?.runId ?? null);
  const isRunning = !!running[sessionId] || activeIds.has(sessionId);
  const lastTurn = timeline?.turns?.at(-1);
  const lastTurnBoundary = lastTurn ? (lastTurn.endSeq ?? lastTurn.startSeq) : null;
  const metadataIsNewer =
    !!lastTurnBoundary &&
    (!latestBoundary || BigInt(lastTurnBoundary) > BigInt(latestBoundary.seq));
  const resumeEnd = metadataIsNewer
    ? lastTurn?.endSeq && ["cancelled", "interrupted", "error"].includes(String(lastTurn.status))
      ? { seq: lastTurn.endSeq }
      : null
    : latestBoundary?.kind === "turn.end" &&
        ["cancelled", "interrupted", "error"].includes(String(latestBoundary.data.status))
      ? { seq: latestBoundary.seq }
      : null;
  const olderLock = useRef(createSessionLoadLock());
  const [timelineLoading, setTimelineLoading] = useState(false),
    [olderTimelineLoading, setOlderTimelineLoading] = useState(false);
  const thinkingVisible =
    running[sessionId] &&
    !stream &&
    !approvals.some((approval) => approval.sessionId === sessionId) &&
    !questions.some((question) => question.sessionId === sessionId);
  // 时间线 ask.request 卡片的回放配对：questionId → 结算数据（跨页翻页时不跨页取）。
  const askAnswers = useMemo(() => {
    const map: Record<string, { answer: string; source: string }> = {};
    for (const item of events)
      if (item.kind === "ask.resolved")
        map[String(item.data.id)] = {
          answer: String(item.data.answer ?? ""),
          source: String(item.data.source ?? ""),
        };
    return map;
  }, [events]);
  // 提问浮层只挂当前会话；其他会话的 pending 提问在侧栏行以「待答复」标记提示。
  const currentQuestion = questions.find((question) => question.sessionId === sessionId);
  // 侧聊入口：父会话事件里的 side 登记是识别“侧聊子会话”的可靠来源（session_get/session_fork 不返回 parentId）。
  const sideSessionIds = useMemo(
    () =>
      new Set(
        events
          .filter((event) => event.kind === "subagent.created" && event.data.source === "side")
          .map((event) => String(event.data.sessionId)),
      ),
    [events],
  );
  // 抽屉的侧聊视图可见时，其审批在抽屉内处理，主窗口弹窗跳过；切走或关闭面板后回落主窗口，提示不丢。
  const chatApprovalVisible = sideOpen && sideView === "chat" && !!chatSessionId;
  const mainApproval =
    approvals.find((approval) => !(chatApprovalVisible && approval.sessionId === chatSessionId)) ??
    null;
  const pendingAskIds = useMemo(
    () => new Set(questions.map((question) => question.sessionId)),
    [questions],
  );
  const skillRows = useMemo(() => skillRowsFrom(skills), [skills]);
  const workspace = workspaces.find((w) => w.id === workspaceId),
    session = sessions.find((s) => s.id === sessionId);
  // 新建会话的默认执行位置：副本项目沿用副本；直接模式与旧 worktree 项目默认直接；
  // 字段缺失按旧记录语义回退副本。
  function defaultExecMode(targetWorkspaceId: string | null | undefined): ExecutionMode {
    const ws = workspaces.find((w) => w.id === targetWorkspaceId);
    if (!ws) return "direct";
    return ws.executionMode === "direct" || ws.executionMode === "worktree" ? "direct" : "replica";
  }
  // 会话有效执行位置：会话固化值 > 已登记 worktree（历史会话）> 项目默认（旧 worktree 按直接处理）。
  function sessionExecMode(
    s: SessionInfo | undefined,
    fallbackWorkspaceId = workspaceId,
  ): ExecutionMode {
    if (s?.executionMode) return s.executionMode;
    const ws = workspaces.find((w) => w.id === (s?.workspaceId || fallbackWorkspaceId));
    if (s && ws?.worktrees?.[s.id]) return "worktree";
    if (!ws) return "direct";
    return ws.executionMode === "direct" || ws.executionMode === "worktree" ? "direct" : "replica";
  }
  const draftExecMode = draftExecOverride ?? defaultExecMode(draftWorkspaceId);
  const draftBucketKey = `${draftWorkspaceId}`;
  // composer 当前编辑值：草稿态读当前落点桶，会话态读 text；写入按态分发到对应存储。
  const composerText = drafting ? (draftBuckets[draftBucketKey] ?? "") : text;
  const canResume =
    !!sessionId &&
    !isRunning &&
    !!resumeEnd &&
    !composerText.trim() &&
    !attachments.length &&
    !quotes.length &&
    !currentQuestion;
  function setComposerText(value: string) {
    if (drafting) setDraftBuckets((old) => ({ ...old, [draftBucketKey]: value }));
    else setText(value);
  }
  const visibleSessions = useMemo(
    () => withoutSubagents(filterByTitle(allSessions, query)),
    [allSessions, query],
  );
  // 子代理面板数据：只取当前会话的子代理会话（草稿态未建会话时为空），运行中的排前由面板处理。
  const panelWorkspaceId = workspaceId || draftWorkspaceId || "";
  const subagentSessions = useMemo(
    () => subagentsOf(allSessions, sessionId),
    [allSessions, sessionId],
  );
  const runningSubagentCount = subagentSessions.reduce(
    (count, s) => count + (activeIds.has(s.id) || running[s.id] ? 1 : 0),
    0,
  );
  // 空串是无项目会话的哨兵工作区（见 no-project-sessions.md）：界面上统一显示「无项目」，
  // 而不是查不到记录时回退成空名字（欢迎语、草稿 chip、占位文案与会话行都用它取名）。
  const projectName = (id: string) =>
    id === "" ? NO_PROJECT_NAME : (workspaces.find((x) => x.id === id)?.name ?? "");
  // 可见项目：隐藏（已移除）项目不进「按项目」分组与草稿候选，其余显示点照常。
  const visibleWorkspaces = useMemo(() => workspaces.filter((w) => !w.hidden), [workspaces]);
  // 同源重复项目：同一源目录存在多条项目记录（常见于数据目录迁移后重开项目）；
  // 隐藏项目也参与检测，用于组头重复标记与「合并同源项目」入口。
  const duplicateSourcePaths = useMemo(() => {
    const counts = new Map<string, number>();
    for (const w of workspaces) counts.set(w.sourcePath, (counts.get(w.sourcePath) ?? 0) + 1);
    return new Set([...counts].filter(([, n]) => n > 1).map(([p]) => p));
  }, [workspaces]);
  // 项目 chip 菜单候选：按搜索词过滤，置顶项目排前，其余按名称排序。
  const pjCandidates = useMemo(
    () =>
      [...visibleWorkspaces]
        .filter((ws) => {
          const q = pjQuery.trim().toLowerCase();
          return !q || ws.name.toLowerCase().includes(q);
        })
        .sort((a, b) => (b.pinned ? 1 : 0) - (a.pinned ? 1 : 0) || a.name.localeCompare(b.name)),
    [visibleWorkspaces, pjQuery],
  );
  const slashToken = commandTokenUnderCursor(composerText, cursor);
  // 输入框可用即弹菜单（含草稿态）；命令级可用性由注册表按是否已建会话过滤。
  const activeSlash =
    slashToken && slashToken !== dismissedToken && (sessionId || drafting) && !sessionCreating
      ? (() => {
          const items = filterSlashCommands(slashToken, { draft: !sessionId });
          return items.length ? { token: slashToken, items } : null;
        })()
      : null;
  const skillToken = useMemo(
    () =>
      (sessionId || drafting) && !sessionCreating
        ? skillTokenUnderCursor(composerText, cursor)
        : null,
    [composerText, cursor, sessionId, drafting, sessionCreating],
  );
  const activeSkill =
    skillToken && skillToken.token !== dismissedToken
      ? (() => {
          const items = filterSkillRows(skillRows, skillToken.token.slice(1));
          return items.length ? { tok: skillToken, items } : null;
        })()
      : null;
  useEffect(() => {
    setSlashIndex(0);
  }, [slashToken, skillToken]);
  // 补全弹层滚动跟随：选中项或候选集变化时把选中项滚入可视区（容器 overflow:auto 不会自动跟随；jsdom 无实现，?. 跳过）。
  useEffect(() => {
    selectedOptionRef.current?.scrollIntoView?.({ block: "nearest" });
  }, [slashIndex, activeSlash, activeSkill]);
  const taskCount = useMemo(() => {
    const open = new Set<string>();
    for (const e of events) {
      const id = e.data?.id;
      if (typeof id !== "string") continue;
      if (e.kind === "job.started") open.add(id);
      else if (e.kind === "job.finished") open.delete(id);
    }
    return open.size;
  }, [events]);
  const generation = useRef(0),
    end = useRef<HTMLDivElement>(null),
    conversationRef = useRef<HTMLElement>(null),
    timelineRequest = useRef(0),
    autoFollow = useRef(true),
    activeWorkspaceId = useRef(workspaceId);
  const creatingSessions = useRef(new Map<string, Promise<SessionInfo>>());
  // 启动判定：首个工作区加载后不再自动打开最近会话（含 StrictMode 对同一工作区的重复执行）。
  const bootstrappedWorkspace = useRef<string | null>(null);
  const lastRunWorkspaceId = useRef<string | null>(null); // workspace effect 实际执行过的最近工作区（null=从未执行）
  activeWorkspaceId.current = workspaceId;
  async function attempt<T>(action: () => Promise<T>): Promise<T | undefined> {
    setError("");
    try {
      return await action();
    } catch (e) {
      setError(errorText(e));
      return undefined;
    }
  }
  useEffect(() => {
    if (!desktop()) return;
    void attempt(async () => {
      const [ws, ms, ps, sel, amode] = await Promise.all([
        command<WorkspaceInfo[]>("workspace_list"),
        command<ModelConfig[]>("model_list"),
        command<ProviderInfo[]>("provider_list"),
        command<DefaultSelection | null>("default_selection_get"),
        command<string>("approval_mode_get"),
      ]);
      setWorkspaces(ws);
      setWorkspacesLoaded(true);
      // 启动恢复上次打开的项目；项目已移除（隐藏）或记录缺失时回退列表首个。
      const lastWorkspace =
        sel?.workspaceId && ws.some((w) => w.id === sel.workspaceId && !w.hidden)
          ? sel.workspaceId
          : (ws[0]?.id ?? "");
      // 启动装载完成：放行 workspace effect 并预置草稿落点（预置与 workspace effect 首跑同批次落定，
      // 不产生中间落点渲染，避免用户首笔输入落入过期分桶）；目标项目有会话时 workspace effect
      // 会随即改打开最近会话。
      setDrafting(true);
      setDraftWorkspaceId(lastWorkspace);
      setDraftExecOverride(null);
      setWorkspaceId(lastWorkspace);
      setModels(ms);
      setProviders(ps);
      setApprovalMode(
        (KNOWN_APPROVALS as readonly string[]).includes(amode) ? (amode as ApprovalPolicy) : "ask",
      );
      // 恢复上次选择；模型或供应商已停用/删除时回退到第一个可用模型，档位回落最高档。
      const usable = ms.filter(
        (m) => m.enabled && ps.some((p) => p.id === m.providerId && p.enabled),
      );
      const saved = sel && usable.some((m) => m.id === sel.modelId) ? sel : null;
      const restored = saved
        ? (usable.find((m) => m.id === saved.modelId) ?? usable[0])
        : usable[0];
      const restoredEffort = restored
        ? saved &&
          saved.modelId === restored.id &&
          saved.effort &&
          restored.efforts.includes(saved.effort)
          ? saved.effort
          : (restored.efforts.at(-1) ?? null)
        : null;
      setModelId(restored?.id ?? "");
      setEffort(restoredEffort);
    });
  }, []);
  // 打开/切换项目即视为使用：跨重启保留，下次启动恢复该项目；启动装载本身不写回（值来自 KV）。
  const lastSavedWorkspace = useRef<string | null>(null);
  useEffect(() => {
    if (!desktop() || !workspaceId) return;
    if (lastSavedWorkspace.current === null) {
      lastSavedWorkspace.current = workspaceId;
      return;
    }
    if (lastSavedWorkspace.current === workspaceId) return;
    lastSavedWorkspace.current = workspaceId;
    void command("default_selection_save", { selection: { modelId, effort, workspaceId } }).catch(
      () => {},
    );
  }, [workspaceId]);
  useEffect(() => {
    // 项目目录尚未加载完成（启动 effect 未就绪）时直接跳过：此时 workspaceId 仍是初始占位空串，
    // 若照常加载，其异步完成会赶在启动加载落地之前，把短暂的「无项目草稿态」写入 UI，
    // 导致启动瞬间草稿落点闪现、用户首笔输入落入错误分桶。
    if (!workspacesLoaded) return;
    const token = ++generation.current,
      targetWorkspaceId = workspaceId;
    const isInitialWorkspaceLoad = lastRunWorkspaceId.current === null;
    const isWorkspaceSwitch =
      !isInitialWorkspaceLoad && lastRunWorkspaceId.current !== targetWorkspaceId;
    lastRunWorkspaceId.current = targetWorkspaceId;
    const firstWorkspaceLoad =
      !!targetWorkspaceId &&
      (bootstrappedWorkspace.current === null ||
        bootstrappedWorkspace.current === targetWorkspaceId);
    if (targetWorkspaceId) bootstrappedWorkspace.current = targetWorkspaceId;
    let disposed = false;
    const current = () =>
      !disposed && token === generation.current && activeWorkspaceId.current === targetWorkspaceId;
    setSessions([]);
    setSessionId("");
    setText("");
    setSessionCreating(false);
    setError("");
    setGoalPending(false);
    // 非桌面环境不加载会话；桌面端空串哨兵（无项目）照常按哨兵过滤加载。
    if (!desktop()) {
      setDrafting(false);
      setDraftWorkspaceId(null);
      return;
    }
    void (async () => {
      try {
        // 侧栏「最近 / 项目」视图需要跨工作区会话；当前工作区列表由全量过滤派生。
        const full = preserveSessionTitleOverrides(
          await command<SessionInfo[]>("session_list", {
            workspaceId: targetWorkspaceId,
            allWorkspaces: true,
          }),
        );
        if (!current()) return;
        setAllSessions(full);
        const list = full.filter((s) => s.workspaceId === targetWorkspaceId);
        if (!list.length) {
          // 无会话的工作区进入草稿态：不自动创建，发送首条消息时才落库。
          // 首次装载时若用户已先进入草稿态（抢在目录装载完成前点了「新建聊天」），保留其落点状态不重写；
          // 之后的工作区切换仍把落点重置为新工作区。
          if (isInitialWorkspaceLoad ? !draftingRef.current : isWorkspaceSwitch) {
            setDrafting(true);
            setDraftWorkspaceId(targetWorkspaceId);
            setDraftExecOverride(null);
          }
          return;
        }
        setSessions(list);
        const jump = pendingJump.current;
        if (jump && list.some((s) => s.id === jump)) {
          pendingJump.current = "";
          setDrafting(false);
          setDraftWorkspaceId(null);
          setSessionId(jump);
        } else if (firstWorkspaceLoad) {
          setDrafting(true);
          setDraftWorkspaceId(targetWorkspaceId);
          setDraftExecOverride(null);
        } else {
          setDrafting(false);
          setDraftWorkspaceId(null);
          setSessionId(list[0]?.id ?? "");
        }
      } catch (e) {
        if (current()) setError(errorText(e));
      }
    })();
    return () => {
      disposed = true;
    };
  }, [workspaceId, workspacesLoaded]);
  const {
    navState,
    navLoading,
    navError,
    navActive,
    jumping,
    loadOlderNav,
    jumpToTurn,
    syncNavPosition,
    retry: retryNav,
    cancelJump,
  } = useTurnNavigation({
    sessionId,
    events,
    activeSessionId,
    autoFollow,
    conversationRef,
    setError,
    loadOlderTimeline,
  });
  useEffect(() => {
    if (!sessionId) return;
    autoFollow.current = true;
    setShowJumpBottom(false);
    void loadLatestTimeline(sessionId);
  }, [sessionId]);
  useEffect(() => {
    const id = sessionId,
      revision = ++planRevision.current;
    setGoalPending(false);
    if (id) setDraftPlan(false);
    setPlanState({ id, enabled: null, busy: false });
    if (!id || !desktop()) return;
    command<boolean>("session_plan_get", { sessionId: id })
      .then((enabled) => {
        if (revision !== planRevision.current || activeSessionId.current !== id) return;
        if (typeof enabled !== "boolean") throw new Error("计划模式状态无效");
        setPlanState({ id, enabled, busy: false });
      })
      .catch((e) => {
        if (revision === planRevision.current && activeSessionId.current === id)
          setError(errorText(e));
      });
    return () => {
      planRevision.current++;
    };
  }, [sessionId]);
  useEffect(() => {
    if (!desktop()) return;
    let alive = true;
    const refresh = () => {
      command<ApprovalRequest[]>("approval_list")
        .then((list) => {
          if (alive) setApprovals(list);
        })
        .catch((e) => {
          if (alive) setError(errorText(e));
        });
      command<AskRequest[]>("question_list")
        .then((list) => {
          if (alive) setQuestions(Array.isArray(list) ? list : []);
        })
        .catch(() => {});
      command<string[]>("session_active_list")
        .then((ids) => {
          if (alive) setActiveIds(new Set(ids));
        })
        .catch(() => {});
    };
    void refresh();
    const timer = setInterval(refresh, 2000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);
  useEffect(() => {
    if (autoFollow.current) end.current?.scrollIntoView?.({ behavior: "smooth" });
  }, [events, stream]);
  // 打开工作区两步走：先选目录，再在命名弹窗确认（名称预填目录名，留空则后端回退目录名）。
  async function openWorkspace() {
    setBusy(true);
    await attempt(async () => {
      const path = await open({ directory: true, multiple: false, title: "选择原项目目录" });
      if (path) setWsDialog({ mode: "create", path, name: basename(path), execMode: "direct" });
    });
    setBusy(false);
  }
  async function createWorkspace() {
    if (!wsDialog || wsDialog.mode !== "create") return;
    const { path, name } = wsDialog;
    setWsDialog(null);
    await attempt(async () => {
      const ws = await command<WorkspaceInfo>("workspace_open", {
        path,
        name: name.trim() || null,
        executionMode: wsDialog.execMode,
      });
      // 重开同源路径项目：重新显示被隐藏的旧项目（会话与自动化原位保留，无隐藏时为 0）。
      const reveal = await command<WorkspaceRevealSummary>("workspace_reveal_hidden", {
        workspaceId: ws.id,
      }).catch(() => null);
      setWorkspaces((old) => [...old.filter((w) => w.id !== ws.id), ws]);
      setWorkspaceId(ws.id);
      if (reveal && reveal.revealed > 0) {
        setNotice(
          `已打开项目「${ws.name}」，重新显示 ${reveal.revealed} 个同目录隐藏项目（其会话与自动化原位保留）。`,
        );
        return;
      }
      setNotice(
        ws.executionMode === "direct"
          ? `已创建直接执行模式工作区「${ws.name}」。改动将直接写入源项目目录。`
          : ws.executionMode === "worktree"
            ? `已创建 worktree 执行模式工作区「${ws.name}」。改动将直接写入会话 worktree 分支。`
            : `已创建托管工作副本「${ws.name}」。原项目仅在确认同步后写入。`,
      );
    });
  }
  async function saveWorkspaceEdit() {
    if (!wsDialog || wsDialog.mode !== "edit") return;
    const { id, name, execMode, currentMode } = wsDialog;
    setWsDialog(null);
    await attempt(async () => {
      const ws = await command<WorkspaceInfo>("workspace_update", {
        workspaceId: id,
        name,
        executionMode: execMode,
      });
      setWorkspaces((old) => old.map((w) => (w.id === ws.id ? ws : w)));
      if (ws.executionMode === currentMode) setNotice(`项目已重命名为「${ws.name}」。`);
      else
        setNotice(
          `项目已保存，默认执行位置已切换为${ws.executionMode === "direct" ? "直接（改动直接写入源项目目录）" : ws.executionMode === "worktree" ? "worktree（改动直接写入会话 worktree 分支）" : "隔离副本（改动经审阅确认后同步）"}，从下一回合起对未固化执行位置的会话生效。`,
        );
    });
  }
  function preserveSessionTitleOverrides(items: SessionInfo[]) {
    return items.map((item) => {
      const title = sessionTitleOverrides.current.get(item.id);
      return title === undefined || title === item.title ? item : { ...item, title };
    });
  }
  function updateSessionInfo(updated: SessionInfo, manual = false) {
    if (manual) {
      manualSessionTitles.current.add(updated.id);
      sessionTitleOverrides.current.set(updated.id, updated.title);
    }
    const title = manualSessionTitles.current.has(updated.id)
      ? (sessionTitleOverrides.current.get(updated.id) ?? updated.title)
      : updated.title;
    sessionTitleOverrides.current.set(updated.id, title);
    const existing =
      allSessions.find((item) => item.id === updated.id) ??
      sessions.find((item) => item.id === updated.id);
    const info = { ...updated, title, parentId: updated.parentId ?? existing?.parentId ?? null };
    const upsertByUpdatedAt = (items: SessionInfo[]) => {
      const index = items.findIndex((item) => item.id === info.id);
      const next =
        index >= 0 ? items.map((item) => (item.id === info.id ? info : item)) : [...items, info];
      return next.sort((a, b) => Date.parse(b.updatedAt) - Date.parse(a.updatedAt));
    };
    setAllSessions(upsertByUpdatedAt);
    setSessions((old) =>
      old.some((item) => item.id === info.id)
        ? upsertByUpdatedAt(old)
        : info.workspaceId === activeWorkspaceId.current
          ? upsertByUpdatedAt(old)
          : old,
    );
  }
  function showSessionContextMenu(event: ReactMouseEvent<HTMLButtonElement>, item: SessionInfo) {
    event.preventDefault();
    setSessionContextMenu({
      sessionId: item.id,
      x: Math.max(4, Math.min(event.clientX, window.innerWidth - 164)),
      y: Math.max(4, Math.min(event.clientY, window.innerHeight - 176)),
    });
  }
  function startSessionTitleGeneration(
    item: SessionInfo,
    prompt: string,
    attachmentNames: string[],
    isFirstMessage: boolean,
  ) {
    if (!isFirstMessage || item.title !== "新会话" || titleGenerationRequests.current.has(item.id))
      return;
    titleGenerationRequests.current.add(item.id);
    void command<SessionInfo>("session_title_generate", {
      sessionId: item.id,
      text: prompt,
      modelId,
      effort,
      attachmentNames,
    })
      .then(updateSessionInfo)
      .catch(() => {})
      .finally(() => titleGenerationRequests.current.delete(item.id));
  }
  function openSessionRename(item: SessionInfo) {
    const dialogId = ++sessionRenameDialogId.current;
    setSessionRename({ id: item.id, name: item.title, dialogId });
    setSessionRenameError("");
  }
  function closeSessionRename() {
    sessionRenameDialogId.current++;
    setSessionRename(null);
    setSessionRenameError("");
  }
  async function saveSessionRename() {
    const draft = sessionRename;
    if (!draft || sessionRenameSaving.current.has(draft.id)) return;
    const title = draft.name.trim();
    if (!title) {
      setSessionRenameError("会话名称不能为空");
      return;
    }
    if (new TextEncoder().encode(title).length > 512) {
      setSessionRenameError("会话名称不得超过512字节");
      return;
    }
    setSessionRenameError("");
    sessionRenameSaving.current.add(draft.id);
    setSessionRenameSavingIds((old) => new Set(old).add(draft.id));
    try {
      const updated = await command<SessionInfo>("session_rename", { sessionId: draft.id, title });
      updateSessionInfo(updated, true);
      if (sessionRenameDialogId.current === draft.dialogId) closeSessionRename();
    } catch (e) {
      if (sessionRenameDialogId.current === draft.dialogId) setSessionRenameError(errorText(e));
    } finally {
      sessionRenameSaving.current.delete(draft.id);
      setSessionRenameSavingIds((old) => {
        if (!old.has(draft.id)) return old;
        const next = new Set(old);
        next.delete(draft.id);
        return next;
      });
    }
  }
  function createSessionFor(
    targetWorkspaceId: string,
    executionMode: ExecutionMode,
  ): Promise<SessionInfo> {
    const key = `${targetWorkspaceId}:${executionMode}`;
    const pending = creatingSessions.current.get(key);
    if (pending) return pending;
    const request = command<SessionInfo>("session_create", {
      workspaceId: targetWorkspaceId,
      title: "新会话",
      executionMode,
    }).then((s) => {
      if (s.workspaceId !== targetWorkspaceId || !s.id)
        throw new Error("创建的会话不属于当前工作区。");
      return s;
    });
    creatingSessions.current.set(key, request);
    const cleanup = () => {
      if (creatingSessions.current.get(key) === request) creatingSessions.current.delete(key);
    };
    void request.then(cleanup, cleanup);
    return request;
  }
  // 新建聊天：进入草稿态（不立即创建会话），落点默认当前项目，自动弹出项目菜单供切换或新建项目。
  function newSession() {
    if (sessionCreating) return;
    // 草稿落点：无项目会话激活（哨兵空串）时默认落「无项目」；纯启动态保持未选（弹出菜单）。
    const target = workspaceId === "" && sessionId ? "" : workspaceId || null;
    setDrafting(true);
    setDraftWorkspaceId(target);
    setDraftExecOverride(null);
    setDraftPlan(false);
    setSessionId("");
    setText("");
    setPjQuery("");
    setPjMenuOpen(true);
  }
  // 项目组头新建聊天：始终直接创建（组头已显式指定落点项目）；非当前项目创建后经 pendingJump 切换定位。
  async function newSessionIn(targetWorkspaceId: string) {
    // 落点允许空串哨兵（无项目组的组头新建）。
    if (sessionCreating) return;
    const token = generation.current;
    const current = () =>
      token === generation.current && activeWorkspaceId.current === targetWorkspaceId;
    if (targetWorkspaceId === workspaceId) {
      setSessionCreating(true);
      setError("");
      setDrafting(false);
      setDraftWorkspaceId(null);
      try {
        const s = await createSessionFor(targetWorkspaceId, defaultExecMode(targetWorkspaceId));
        setAllSessions((old) => [s, ...old.filter((item) => item.id !== s.id)]);
        if (!current()) return;
        setSessions((old) => [s, ...old.filter((item) => item.id !== s.id)]);
        setSessionId(s.id);
        setText("");
      } catch (e) {
        if (current()) setError(errorText(e));
      } finally {
        if (current()) setSessionCreating(false);
      }
      return;
    }
    await attempt(async () => {
      const s = await createSessionFor(targetWorkspaceId, defaultExecMode(targetWorkspaceId));
      pendingJump.current = s.id;
      setWorkspaceId(targetWorkspaceId);
    });
  }
  async function addAttachment() {
    const id = sessionId,
      workspace = workspaceId,
      epoch = attachmentEpoch.current;
    if (!id || importing) return;
    setImporting(true);
    const current = () =>
      epoch === attachmentEpoch.current &&
      activeSessionId.current === id &&
      activeWorkspaceId.current === workspace;
    try {
      const path = await open({
        multiple: false,
        directory: false,
        title: "选择附件（最多20MiB）",
      });
      if (!path || !current()) return;
      const artifact = await command<ArtifactMetadata>("artifact_import", { sessionId: id, path });
      if (current())
        setAttachments((old) =>
          old.some((a) => a.hash === artifact.hash) ? old : [...old, artifact],
        );
    } catch (e) {
      if (current()) setError(errorText(e));
    } finally {
      if (current()) setImporting(false);
    }
  }
  // 切换模型：当前档位被新模型支持则保留，否则回落其最高档（默认档位即最高档）；
  // 同时静默持久化默认选择，并在会话流插入临时切换标记（不落库）。
  function chooseModel(id: string) {
    const next = models.find((m) => m.id === id);
    const kept =
      next && next.efforts.length
        ? effort && next.efforts.includes(effort)
          ? effort
          : (next.efforts.at(-1) ?? null)
        : null;
    const from = models.find((m) => m.id === modelId);
    setModelId(id);
    setEffort(kept);
    void command("default_selection_save", {
      selection: { modelId: id, effort: kept, workspaceId: workspaceId || null },
    }).catch(() => {});
    if (from && from.id !== id && sessionId)
      addModelNote(
        `模型已切换：${from.name || from.model} → ${next?.name || next?.model || id} · 档位 ${kept ?? "默认"}`,
      );
  }
  function chooseEffort(value: string) {
    const next = value === "" ? null : value;
    setEffort(next);
    void command("default_selection_save", {
      selection: { modelId, effort: next, workspaceId: workspaceId || null },
    }).catch(() => {});
  }
  function addModelNote(text: string) {
    const id = sessionId;
    if (!id) return;
    // 锚定切换时刻时间线最后一条事件的 seq：标记固定在切换点，后续回复渲染在标记之后，不再跟随最新回复。
    const anchorSeq = useChat.getState().events[id]?.at(-1)?.seq ?? "0";
    const noteId = `model-note-${++modelNoteSeq.current}`;
    setModelNotes((old) => [...old, { id: noteId, sessionId: id, text, anchorSeq }]);
  }
  // 权限模式：保存为默认档并对当前会话即时生效；会话内显式切换记录覆盖（与其他会话隔离）。
  const approvalModeForSession = (id: string) => approvalOverrides[id] ?? approvalMode;
  function chooseApprovalMode(mode: ApprovalPolicy) {
    setApprovalMode(mode);
    if (sessionId) setApprovalOverrides((old) => ({ ...old, [sessionId]: mode }));
    void attempt(async () => {
      await command("approval_mode_save", { mode, sessionId: sessionId || null });
    });
  }
  async function setPlanMode(id: string, enabled: boolean): Promise<boolean> {
    if (!id || planPending.current.has(id)) return false;
    planPending.current.add(id);
    ++planRevision.current;
    if (activeSessionId.current === id)
      setPlanState((old) => ({ id, enabled: old.id === id ? old.enabled : null, busy: true }));
    setError("");
    try {
      await command("session_plan_set", { sessionId: id, enabled });
      if (activeSessionId.current === id) {
        setPlanState({ id, enabled, busy: false });
        setNotice(enabled ? "已进入计划模式。" : "已退出计划模式。");
      }
      return true;
    } catch (e) {
      if (activeSessionId.current === id) {
        setPlanState({ id, enabled: null, busy: false });
        setError(errorText(e));
      } else if (!activeSessionId.current) setError(errorText(e));
      return false;
    } finally {
      planPending.current.delete(id);
      if (activeSessionId.current === id)
        setPlanState((old) => (old.id === id ? { ...old, busy: false } : old));
    }
  }
  async function togglePlanMode() {
    if (!sessionId) {
      if (drafting) {
        setDraftPlan((old) => !old);
        setNotice(draftPlan ? "草稿已取消计划模式。" : "草稿将在发送时启用计划模式。");
      }
      return;
    }
    if (planState.id !== sessionId || planState.enabled === null || planState.busy) return;
    await setPlanMode(sessionId, !planState.enabled);
  }
  async function loadLatestTimeline(id: string) {
    const request = ++timelineRequest.current;
    if (activeSessionId.current === id) {
      setTimelineLoading(true);
      setOlderTimelineLoading(false);
    }
    useChat.getState().activateTimeline(id);
    try {
      const page = await command<TimelinePage>("session_timeline_page", { sessionId: id });
      if (request !== timelineRequest.current || activeSessionId.current !== id) return;
      const existing = useChat.getState().timeline(id);
      if (!autoFollow.current && existing?.pages.some((p) => p.length)) {
        const pages = [page],
          through = BigInt(existing.throughSeq);
        let oldest = page;
        while (oldest.events.length && BigInt(oldest.events[0].seq) > through + 1n) {
          if (!oldest.hasOlder || !oldest.olderCursor)
            throw new Error("无法补齐最新消息，请重试。");
          const before = oldest.olderCursor;
          oldest = await command<TimelinePage>("session_timeline_page", {
            sessionId: id,
            beforeSeq: before,
          });
          if (request !== timelineRequest.current || activeSessionId.current !== id) return;
          if (!oldest.events.length || oldest.olderCursor === before)
            throw new Error("无法补齐最新消息，请重试。");
          pages.push(oldest);
        }
        useChat.getState().mergeLatestTimelinePages(id, pages);
      } else useChat.getState().replaceTimelinePage(id, page);
    } catch (e) {
      if (request === timelineRequest.current && activeSessionId.current === id)
        setError(errorText(e));
    } finally {
      if (request === timelineRequest.current && activeSessionId.current === id)
        setTimelineLoading(false);
    }
  }
  async function loadOlderTimeline(): Promise<boolean> {
    const id = sessionId;
    return olderLock.current.run(id, async () => {
      const current = useChat.getState().timeline(id);
      if (!id || !current?.hasOlder || !current.olderCursor) return false;
      const request = ++timelineRequest.current,
        node = conversationRef.current,
        height = node?.scrollHeight ?? 0,
        top = node?.scrollTop ?? 0,
        before = current.olderCursor;
      autoFollow.current = false;
      setOlderTimelineLoading(true);
      const pending = (async () => {
        try {
          const page = await command<TimelinePage>("session_timeline_page", {
            sessionId: id,
            beforeSeq: before,
          });
          if (
            request !== timelineRequest.current ||
            activeSessionId.current !== id ||
            useChat.getState().timeline(id)?.olderCursor !== before
          )
            return false;
          useChat.getState().prependTimelinePage(id, page);
          requestAnimationFrame(() => {
            if (node && activeSessionId.current === id)
              node.scrollTop = top + node.scrollHeight - height;
          });
          return true;
        } catch (e) {
          if (request === timelineRequest.current && activeSessionId.current === id)
            setError(errorText(e));
          return false;
        } finally {
          if (request === timelineRequest.current && activeSessionId.current === id)
            setOlderTimelineLoading(false);
        }
      })();
      return pending;
    });
  }
  // 一键回底：恢复自动跟随并平滑滚动到最新内容；此后流式追加由自动跟随接管。
  function jumpToBottom() {
    const node = conversationRef.current;
    if (!node) return;
    autoFollow.current = true;
    setShowJumpBottom(false);
    node.scrollTo({ top: node.scrollHeight, behavior: "smooth" });
  }
  function receiveEvent(event: LiveEvent) {
    useChat.getState().receive(event);
    if (
      event.type === "durable" &&
      event.event.kind === "session.plan" &&
      event.event.sessionId === activeSessionId.current &&
      typeof event.event.data.enabled === "boolean"
    ) {
      ++planRevision.current;
      setPlanState({
        id: event.event.sessionId,
        enabled: event.event.data.enabled,
        busy: planPending.current.has(event.event.sessionId),
      });
    }
    // 计划抽屉联动：plan_write 写入计划后自动弹出「计划」视图（仿网页视图先例）；
    // 待办/目标只驱动刷新。事件仅来自当前交互会话（session_send 按会话过滤）。
    if (event.type === "durable" && PLAN_STATE_KINDS.includes(event.event.kind)) {
      if (event.event.kind === "todo.state") noteLiveStateEvent(event.event);
      if (event.event.kind === "plan.state") useSidePanel.getState().openPlans();
      setPlanPanelRevision((v) => v + 1);
    }
  }
  async function dispatchText(
    id: string,
    input: string,
    hashes: string[],
    messageQuotes: string[] = [],
  ) {
    setRunning((old) => ({ ...old, [id]: true }));
    await attempt(async () => {
      // 无引用时保持原参数个数，避免既有 send 断言与调用形态漂移。
      if (messageQuotes.length && hashes.length)
        await send(id, input, modelId, receiveEvent, hashes, effort, messageQuotes);
      else if (messageQuotes.length)
        await send(id, input, modelId, receiveEvent, undefined, effort, messageQuotes);
      else if (hashes.length) await send(id, input, modelId, receiveEvent, hashes, effort);
      else await send(id, input, modelId, receiveEvent, undefined, effort);
    });
    setRunning((old) => ({ ...old, [id]: false }));
    useChat.getState().clearStream(id);
    if (activeSessionId.current === id) await loadLatestTimeline(id);
  }
  async function submit(e: FormEvent) {
    e.preventDefault();
    if (canResume && resumeEnd && !sessionCreating && !importing && !planState.busy && modelId) {
      const id = sessionId;
      setRunning((old) => ({ ...old, [id]: true }));
      await attempt(async () => {
        await resume(id, resumeEnd.seq, modelId, receiveEvent, effort);
      });
      setRunning((old) => ({ ...old, [id]: false }));
      useChat.getState().clearStream(id);
      if (activeSessionId.current === id) await loadLatestTimeline(id);
      return;
    }
    // 提问挂起时发送即答复：无附件/引用时以原始文本按 text 来源结算提问，不重复入队（与 CLI REPL 一致）；带附件/引用时仍走消息排队。
    if (currentQuestion && !attachments.length && !quotes.length) {
      const answer = text.trim();
      if (!answer) return;
      void resolveQuestion(currentQuestion.id, { text: answer, source: "text" });
      setText("");
      return;
    }
    if (
      sessionCreating ||
      importing ||
      (!composerText.trim() && !attachments.length && !quotes.length) ||
      planPending.current.has(sessionId)
    )
      return;
    // /side 命中即派生：命令本身不进对话（也不作运行中转向），文字作为侧聊首条消息发送。
    const sideMatch = parseSideSlashCommand(composerText);
    if (sideMatch) {
      await startSideConversation(sideMatch.text);
      return;
    }
    if (isRunning && !currentQuestion) {
      if (steerBusy.current) return;
      if (!activeRunId) {
        setError("当前回合尚未就绪或已结束，请稍后重试。");
        return;
      }
      const id = sessionId,
        input = composerText,
        hashes = attachments.map((a) => a.hash),
        messageQuotes = quotes;
      const key = JSON.stringify([id, activeRunId, input, hashes, messageQuotes]);
      if (steerRetry.current?.key !== key) steerRetry.current = { key, id: crypto.randomUUID() };
      const messageId = steerRetry.current.id;
      steerBusy.current = true;
      setError("");
      try {
        await command("session_steer", {
          sessionId: id,
          expectedRunId: activeRunId,
          messageId,
          text: input,
          attachments: hashes,
          quotes: messageQuotes,
        });
        steerRetry.current = null;
        if (activeSessionId.current === id) {
          setText((old) => (old === input ? "" : old));
          setAttachments((old) => old.filter((a) => !hashes.includes(a.hash)));
          setQuotes((old) => old.filter((q) => !messageQuotes.includes(q)));
          await loadLatestTimeline(id);
        }
      } catch (e) {
        if (activeSessionId.current === id) setError(errorText(e));
      } finally {
        steerBusy.current = false;
      }
      return;
    }
    const planMatch = /^\/plan(?:\s+([\s\S]+))?$/.exec(composerText.trim());
    const planTask = planMatch?.[1]?.trim();
    if (planMatch && !planTask) {
      await togglePlanMode();
      if (!sessionId || planState.enabled !== null) setComposerText("");
      return;
    }
    if (!modelId) return;
    const expanded = planMatch ? null : expandSlashPrompt(composerText);
    const goalText =
      goalPending && !expanded && !composerText.trim().startsWith("/")
        ? `${GOAL_PROMPT}\n\n目标：${composerText.trim()}`
        : "";
    if (expanded?.kind === "usage") {
      setComposerText("");
      setGoalPending(true);
      setNotice("目标模式已开启：下一条发送的消息将作为本会话目标。");
      return;
    }
    const input = planTask ?? (expanded ? expanded.text : goalText || composerText),
      hashes = attachments.map((a) => a.hash),
      attachmentNames = attachments.map((a) => a.name);
    if (sessionId) {
      const id = sessionId,
        selectionVersion = selectionEpoch.current.version;
      if (planMatch) {
        if (planState.id !== id || planState.enabled === null || planState.busy) return;
        if (
          !(await setPlanMode(id, true)) ||
          activeSessionId.current !== id ||
          selectionEpoch.current.version !== selectionVersion
        )
          return;
      }
      if (session)
        startSessionTitleGeneration(
          session,
          input,
          attachmentNames,
          !events.some((event) => event.kind === "user.message"),
        );
      setText("");
      setGoalPending(false);
      setAttachments([]);
      const messageQuotes = quotes;
      setQuotes([]);
      await dispatchText(id, input, hashes, messageQuotes);
      return;
    }
    // 草稿态：发送首条消息时才创建会话；落点项目非当前工作区时经 pendingJump 切换定位。
    if (!drafting || draftWorkspaceId === null) return;
    const target = draftWorkspaceId,
      fromWorkspace = workspaceId,
      fromSession = sessionId,
      selectionVersion = selectionEpoch.current.version;
    setSessionCreating(true);
    setError("");
    try {
      const s = await createSessionFor(target, draftExecMode);
      setAllSessions((old) => [s, ...old.filter((item) => item.id !== s.id)]);
      if (planMatch || draftPlan) {
        const enabled = await setPlanMode(s.id, true);
        const stillDraft =
          activeWorkspaceId.current === fromWorkspace &&
          activeSessionId.current === fromSession &&
          selectionEpoch.current.version === selectionVersion;
        if (!enabled) {
          if (stillDraft) {
            setDrafting(false);
            setDraftWorkspaceId(null);
            setDraftPlan(false);
            setText(draftBuckets[`${target}`] ?? "");
            setDraftBuckets((old) => ({ ...old, [`${target}`]: "" }));
            if (target !== fromWorkspace) {
              pendingJump.current = s.id;
              setWorkspaceId(target);
            } else {
              setSessions((old) => [s, ...old.filter((item) => item.id !== s.id)]);
              setSessionId(s.id);
            }
          }
          return;
        }
        setDraftBuckets((old) => ({ ...old, [`${target}`]: "" }));
        if (!stillDraft) return;
      }
      setDraftPlan(false);
      setGoalPending(false);
      startSessionTitleGeneration(s, input, attachmentNames, true);
      setDraftBuckets((old) => ({ ...old, [`${target}`]: "" }));
      setDrafting(false);
      setDraftWorkspaceId(null);
      setText("");
      setAttachments([]);
      const messageQuotes = quotes;
      setQuotes([]);
      // 迟到守卫：发送期间用户已切走（换会话/换工作区）则不再抢占 UI，消息仍归档到新会话。
      // 经 ref 读实时值（闭包快照恒等于 from*，守不住 await 期间的切换）。
      if (activeWorkspaceId.current === fromWorkspace && activeSessionId.current === fromSession) {
        if (target !== fromWorkspace) {
          pendingJump.current = s.id;
          setWorkspaceId(target);
        } else {
          setSessions((old) => [s, ...old.filter((item) => item.id !== s.id)]);
          setSessionId(s.id);
        }
      }
      // 会话已就绪：创建态到此解除，首轮执行交由 running[s.id] 表达（finally 仍兜底提前返回路径）。
      setSessionCreating(false);
      await dispatchText(s.id, input, hashes, messageQuotes);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setSessionCreating(false);
    }
  }
  // 目标项插入命令，用户补齐参数后发送。
  function insertPlusCommand(name: "goal") {
    setPlusOpen(false);
    const next = `/${name} `;
    setComposerText(next);
    setCursor(next.length);
    setDismissedToken("");
    textareaRef.current?.focus();
    requestAnimationFrame(() => {
      textareaRef.current?.setSelectionRange(next.length, next.length);
    });
  }
  function applyCompletion(spec: SlashSpec, token: string) {
    const rest = composerText.slice(token.length);
    const next = `/${spec.name}${rest || " "}`;
    setComposerText(next);
    setCursor(next.length);
    setDismissedToken("");
    requestAnimationFrame(() => {
      textareaRef.current?.setSelectionRange(next.length, next.length);
    });
  }
  // 技能补全：替换光标处 $token 为 $名称（token 后已有空格不重复补），同一 token 内不再弹窗。
  function applySkillCompletion(row: SkillRow, tok: SkillToken) {
    const completed = `$${row.name}`;
    const needsSpace = composerText[tok.end] !== " ";
    const next =
      composerText.slice(0, tok.start) +
      completed +
      (needsSpace ? " " : "") +
      composerText.slice(tok.end);
    const caret = tok.start + completed.length + (needsSpace ? 1 : 0);
    setComposerText(next);
    setCursor(caret);
    setDismissedToken(`$${row.name}`);
    requestAnimationFrame(() => {
      textareaRef.current?.setSelectionRange(caret, caret);
    });
  }
  // /side（别名 /btw）：从当前会话派生侧聊——父会话历史仅供模型参考，侧聊在右侧抽屉从空白开始；
  // 带文字时作为侧聊首条消息发送。主会话不切换、不清附件与引用，侧聊挂在父会话名下。
  async function startSideConversation(firstText?: string) {
    if (!sessionId) {
      setNotice("侧聊需要已有会话；请先发送一条消息或从侧栏打开会话。");
      return;
    }
    if (sessionCreating || running[sessionId]) {
      setNotice("会话运行中，请等回合结束后再派生侧聊。");
      return;
    }
    if (session?.parentId) {
      setNotice("侧聊不能再派生侧聊；请回到主会话操作。");
      return;
    }
    setComposerText("");
    setCursor(0);
    setDismissedToken("");
    try {
      const side = await command<SessionInfo>("session_fork", { sessionId, asSide: true });
      // session_fork 返回值不带 parentId：补齐后再入列表，避免侧聊被当成主会话显示在侧栏。
      const sideInfo = { ...side, parentId: side.parentId ?? sessionId };
      setSessions((old) => (old.some((s) => s.id === sideInfo.id) ? old : [sideInfo, ...old]));
      setAllSessions((old) => upsertSession(old, sideInfo));
      useSidePanel.getState().openChat(sideInfo.id, sessionId, firstText);
      setNotice(`已创建侧聊「${sideInfo.title}」，已在右侧打开；主会话保持不变。`);
    } catch (e) {
      setError(errorText(e));
    }
  }
  // 侧聊入口统一走抽屉；父会话取当前主会话（登记卡/子代理面板都在父会话内触发）。
  function openSideChat(childId: string, parentId: string) {
    useSidePanel.getState().openChat(childId, parentId);
  }
  async function executeSlash(spec: SlashSpec) {
    setComposerText("");
    setCursor(0);
    setDismissedToken("");
    if (spec.name === "help") {
      setNotice(
        `可用命令：${SLASH_COMMANDS.map((item) => `/${item.name}（${item.description}）`).join("、")}。输入 / 唤出补全。`,
      );
      return;
    }
    if (spec.name === "status") {
      setNotice(
        `会话 ${sessionId || "（新会话草稿）"} · 模型 ${models.find((item) => item.id === modelId)?.name ?? modelId} · 工作副本已隔离，改动需审阅后同步。`,
      );
      return;
    }
    if (spec.name === "model") {
      setModelMenuOpen(true);
      return;
    }
    if (spec.name === "diff") {
      useSidePanel.getState().openChanges();
      return;
    }
    if (spec.name === "plan") {
      await togglePlanMode();
      return;
    }
    if (spec.name === "goal") {
      setGoalPending(true);
      setNotice("目标模式已开启：下一条发送的消息将作为本会话目标。");
      return;
    }
    if (spec.name === "clear") {
      await newSession();
      return;
    }
    // /side 先于通用守卫：运行中/模型未就绪也要经 startSideConversation 给出明确提示。
    if (spec.name === "side") {
      await startSideConversation();
      return;
    }
    if (!sessionId || sessionCreating || running[sessionId] || !modelId) return;
    if (spec.name === "init") {
      if (session)
        startSessionTitleGeneration(
          session,
          INIT_PROMPT,
          [],
          !events.some((event) => event.kind === "user.message"),
        );
      await dispatchText(sessionId, INIT_PROMPT, []);
      setNotice("已发起 AGENTS.md 生成；完成后用“查看更改”审阅并同步到原项目。");
      return;
    }
    if (spec.name === "compact") {
      setNotice("正在压缩会话历史……");
      await attempt(async () => {
        await command("session_compact", { sessionId, modelId, effort });
        await loadLatestTimeline(sessionId);
        setNotice("历史已压缩为摘要，后续回合将基于摘要继续。");
      });
      return;
    }
  }
  async function cancel() {
    await attempt(() => command("session_cancel", { sessionId }));
  }
  // 子代理面板的停止按钮：session_cancel 支持任意会话，运行状态由 activeIds 轮询收敛。
  async function cancelSubagent(id: string) {
    await attempt(() => command("session_cancel", { sessionId: id }));
  }
  async function openSession(id: string) {
    if (id === sessionId) return;
    await attempt(async () => {
      let target = sessions.find((s) => s.id === id);
      if (!target) {
        target = (await command<SessionInfo[]>("session_list", { workspaceId })).find(
          (s) => s.id === id,
        );
      }
      if (!target) return;
      if (target.workspaceId !== workspaceId) throw new Error("子会话不属于当前工作区。");
      setSessions((old) => (old.some((s) => s.id === target!.id) ? old : [target!, ...old]));
      setSessionId(target.id);
      setDrafting(false);
      setDraftWorkspaceId(null);
      setText("");
    });
  }
  function openFromPanel(id: string, workspace: string) {
    setPanel(null);
    if (workspace === workspaceId) {
      void openSession(id);
      return;
    }
    pendingJump.current = id;
    setWorkspaceId(workspace);
  }
  // 侧栏跨工作区点击：同工作区直接打开，跨工作区先切工作区再经 pendingJump 跳转。
  function openFromSidebar(s: SessionInfo) {
    if (s.workspaceId === workspaceId) {
      void openSession(s.id);
      setText("");
      return;
    }
    openFromPanel(s.id, s.workspaceId);
  }
  // 项目组头右键「移除项目」：确认后 workspace_remove；有运行中会话时按钮为「移除并停止运行」。
  function openRemoveConfirm(ws: WorkspaceInfo) {
    setWsContextMenu(null);
    const hasRunning = allSessions.some(
      (s) => s.workspaceId === ws.id && (running[s.id] || activeIds.has(s.id)),
    );
    setWsRemove({ id: ws.id, name: ws.name, running: hasRunning });
  }
  async function removeWorkspace() {
    const target = wsRemove;
    if (!target) return;
    try {
      const summary = await command<WorkspaceRemovalSummary>("workspace_remove", {
        workspaceId: target.id,
        stopRunning: target.running,
      });
      // 隐藏语义：项目记录与会话保留，仅置 hidden 标志；「最近」视图会话照常可达。
      setWorkspaces((old) => old.map((w) => (w.id === target.id ? { ...w, hidden: true } : w)));
      setCollapsed((old) => old.filter((x) => x !== target.id));
      if (workspaceId === target.id || workspaces.find((w) => w.id === workspaceId)?.hidden) {
        const next = visibleWorkspaces.find((w) => w.id !== target.id);
        pendingJump.current = "";
        setWorkspaceId(next?.id ?? "");
        setSessionId("");
      }
      setWsRemove(null);
      setNotice(`已移除项目「${target.name}」，数据全部保留；重新打开同一目录的项目会重新显示它。`);
    } catch (e) {
      setError(errorText(e));
    }
  }
  // 项目组头右键「合并同源项目」：保留方自动固定为创建时间最早的项目
  // （workspaces 按后端 WorkspaceManager::list 的 created_at 升序返回，同源子集
  // 首项即最早登记，通常是迁移前的原始项目），其余同源项目全部并入。
  function openMergeConfirm(ws: WorkspaceInfo) {
    setWsContextMenu(null);
    const keep = workspaces.filter((w) => w.sourcePath === ws.sourcePath)[0] ?? ws;
    setWsMergeKeepId(keep.id);
    setWsMerge({ sourcePath: ws.sourcePath });
  }
  // 合并同源重复项目：保留方之外的同源项目逐个并入（会话/自动化/定时任务改挂，
  // 记录目录删除），完成后整体重拉项目与会话列表（归属变化跨两个项目）。
  async function mergeWorkspace() {
    const merge = wsMerge;
    if (!merge) return;
    const keepId = wsMergeKeepId;
    const froms = workspaces.filter((w) => w.sourcePath === merge.sourcePath && w.id !== keepId);
    if (!froms.length) {
      setWsMerge(null);
      return;
    }
    try {
      let movedSessions = 0,
        movedAutomations = 0,
        leftovers: string[] = [];
      for (const from of froms) {
        const stopRunning = allSessions.some(
          (s) => s.workspaceId === from.id && (running[s.id] || activeIds.has(s.id)),
        );
        const summary = await command<WorkspaceMergeSummary>("workspace_merge", {
          fromWorkspaceId: from.id,
          toWorkspaceId: keepId,
          stopRunning,
        });
        movedSessions += summary.movedSessions;
        movedAutomations += summary.movedAutomations;
        leftovers = [...leftovers, ...summary.leftoverWorktrees];
      }
      const [ws, list] = await Promise.all([
        command<WorkspaceInfo[]>("workspace_list"),
        command<SessionInfo[]>("session_list", { workspaceId: keepId, allWorkspaces: true }),
      ]);
      setWorkspaces(ws);
      setAllSessions(preserveSessionTitleOverrides(list));
      if (workspaceId && froms.some((w) => w.id === workspaceId)) setWorkspaceId(keepId);
      setWsMerge(null);
      setNotice(
        `已合并 ${froms.length} 个重复项目：${movedSessions} 个会话与 ${movedAutomations} 个自动化任务挂到保留项目。${leftovers.length ? `源项目下仍有 ${leftovers.length} 个会话 worktree 残留，请手动清理。` : ""}`,
      );
    } catch (e) {
      setError(errorText(e));
    }
  }
  // 存档会话：后端落库后从任务列表移入归档集合；当前会话被存档时统一回到该项目的新会话草稿态。
  async function archiveSession(item: SessionInfo) {
    // 存档会从列表移除该行，光标无从触发 mouseLeave，须主动收起悬停浮层避免驻留。
    dismissSessionPop();
    try {
      const updated = await command<SessionInfo>("session_archive", { sessionId: item.id });
      setAllSessions((old) => old.filter((s) => s.id !== item.id));
      setSessions((old) => old.filter((s) => s.id !== item.id));
      setArchivedSessions((old) => [updated, ...old.filter((s) => s.id !== item.id)]);
      // 统一回主页：不自动打开同项目其他会话。allSessions 含侧栏隐藏的子代理会话，
      // 按「下一个会话」切换会误载其内容（如被存档会话的子代理），因此只清空当前会话进入草稿态。
      if (sessionId === item.id) {
        setSessionId("");
        setText("");
        setDrafting(!!workspaceId || sessionId === "");
        setDraftWorkspaceId(workspaceId || null);
        setDraftExecOverride(null);
        setDraftPlan(false);
      }
      setNotice(`已存档会话「${item.title}」。`);
    } catch (e) {
      setError(errorText(e));
    }
  }
  // 从设置页「存档管理」恢复会话：后端落库后插回列表（按更新时间排序）。
  async function restoreSession(id: string) {
    try {
      const updated = await command<SessionInfo>("session_restore", { sessionId: id });

      setArchivedSessions((old) => old.filter((s) => s.id !== id));
      const upsert = (items: SessionInfo[]) =>
        [...items.filter((s) => s.id !== id), updated].sort(
          (a, b) => Date.parse(b.updatedAt) - Date.parse(a.updatedAt),
        );
      setAllSessions(upsert);
      if (updated.workspaceId === workspaceId)
        setSessions((old) => (old.some((s) => s.id === id) ? old : upsert(old)));
      setNotice(`已恢复会话「${updated.title}」。`);
    } catch (e) {
      setError(errorText(e));
    }
  }
  // 置顶切换：乐观更新，失败回滚并提示。
  async function togglePinned(target: WorkspaceInfo) {
    const next = !target.pinned;
    setWorkspaces((old) => old.map((w) => (w.id === target.id ? { ...w, pinned: next } : w)));
    try {
      await command<WorkspaceInfo>("workspace_set_pinned", {
        workspaceId: target.id,
        pinned: next,
      });
    } catch (e) {
      setWorkspaces((old) => old.map((w) => (w.id === target.id ? { ...w, pinned: !next } : w)));
      setError(errorText(e));
    }
  }
  // 打开设置页：同时刷新归档会话列表（存档管理数据源）；可指定初始分类（如从发送框「管理模型」进入模型设置）。
  function openSettings(section?: SettingsCategoryId) {
    setSettingsSection(section ?? null);
    setSettingsOpen(true);
    command<SessionInfo[]>("session_list", { workspaceId, allWorkspaces: true, archived: true })
      .then((list) => setArchivedSessions(list))
      .catch(() => {});
  }
  // 模型/供应商保存后刷新目录并做默认选中回退（模型设置保存后由设置页回调）。
  async function refreshModels() {
    const [ms, ps] = await Promise.all([
      command<ModelConfig[]>("model_list"),
      command<ProviderInfo[]>("provider_list"),
    ]);
    setModels(ms);
    setProviders(ps);
    if (!ms.some((m) => m.id === modelId) || !ms.find((m) => m.id === modelId)?.enabled) {
      const usable = ms.filter(
        (m) => m.enabled && ps.some((p) => p.id === m.providerId && p.enabled),
      );
      setModelId(usable[0]?.id ?? "");
      setEffort(usable[0]?.efforts.at(-1) ?? null);
    }
  }
  function chooseView(mode: ViewMode) {
    setViewMode(mode);
    try {
      localStorage.setItem("sessionView", mode);
    } catch {}
  }
  // 侧栏宽度拖拽：手写 pointer 拖拽并记忆（与右侧面板同模式），宽度限制 200px ~ 40% 视口。
  function startSidebarDrag(e: React.PointerEvent<HTMLDivElement>) {
    e.preventDefault();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const max = Math.min(window.innerWidth * 0.4, 480);
      setSidebarWidth(Math.min(Math.max(SIDEBAR_MIN_WIDTH, ev.clientX), max));
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      try {
        localStorage.setItem(SIDEBAR_WIDTH_KEY, String(sidebarWidthRef.current));
      } catch {
        /* 隐私模式等场景忽略 */
      }
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  }
  function toggleCollapsed(id: string) {
    setCollapsed((old) => {
      const next = old.includes(id) ? old.filter((x) => x !== id) : [...old, id];
      try {
        localStorage.setItem("sessionCollapsed", JSON.stringify(next));
      } catch {}
      return next;
    });
  }
  async function resolve(id: string, allow: boolean) {
    await attempt(async () => {
      await command("approval_resolve", { id, allow });
      setApprovals((old) => old.filter((a) => a.id !== id));
    });
  }
  async function resolveQuestion(id: string, answer: AskAnswer) {
    await attempt(async () => {
      await command("question_resolve", { id, answer });
      setQuestions((old) => old.filter((q) => q.id !== id));
    });
  }
  async function exportSession(targetSessionId: string) {
    await attempt(async () => {
      const path = await save({
        defaultPath: "rust-harness-session.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      await command("session_export", { sessionId: targetSessionId, path });
      if (exportNoticeTimer.current !== null) clearTimeout(exportNoticeTimer.current);
      setNotice("会话已导出。");
      exportNoticeTimer.current = setTimeout(() => {
        setNotice((current) => (current === "会话已导出。" ? "" : current));
        exportNoticeTimer.current = null;
      }, 3000);
    });
  }
  return (
    <div className="app" aria-hidden={settingsOpen || undefined}>
      <SessionSidebar
        {...{
          sidebarWidth,
          newSession,
          sessionCreating,
          query,
          setQuery,
          viewMode,
          chooseView,
          dismissSessionPop,
        }}
        {...{
          visibleSessions,
          workspaces,
          sessionId,
          openFromSidebar,
          showSessionContextMenu,
          openSessionPop,
          scheduleSessionPopClose,
          pendingAskIds,
        }}
        {...{
          projectName,
          running,
          activeIds,
          archiveSession,
          visibleWorkspaces,
          collapsed,
          setWsContextMenu,
          toggleCollapsed,
        }}
        {...{
          duplicateSourcePaths,
          newSessionIn,
          togglePinned,
          workspaceId,
          openWorkspace,
          busy,
          setPanel,
          automationCount,
        }}
        {...{ openSettings, startSidebarDrag }}
      />
      {sessionPop &&
        createPortal(
          <SessionPopCard title={sessionPop.title} time={sessionPop.time} rect={sessionPop.rect} />,
          document.body,
        )}
      <main className="main">
        <header className="topbar">
          <div className="breadcrumb">
            <strong className="topbar-session-title">
              {session?.title ?? (drafting ? "新会话" : "新的可能，从这里开始")}
            </strong>
            <span className="workspace-context">
              <Folder size={15} />
              <span className="workspace-name">
                {workspaceId === "" ? NO_PROJECT_NAME : workspace ? workspace.name : "工作台"}
              </span>
            </span>
          </div>
          <div className="toolbar">
            <button
              className="soft"
              aria-expanded={panel === "tasks"}
              aria-controls="task-popover"
              onClick={() => setPanel((p) => (p === "tasks" ? null : "tasks"))}
              disabled={!sessionId}
            >
              <ListTodo size={16} />
              后台任务{taskCount > 0 && <span className="count">{taskCount}</span>}
            </button>
            <button
              className="soft"
              aria-expanded={sideOpen}
              onClick={() => useSidePanel.getState().toggle()}
              disabled={!workspaceId && !sessionId}
            >
              <GitCompareArrows size={16} />
              查看更改
            </button>
            <button
              className="archive-button"
              type="button"
              title="存档会话"
              aria-label="存档会话"
              disabled={!sessionId}
              onClick={() => {
                if (session) archiveSession(session);
              }}
            >
              <Archive size={16} />
              存档
            </button>
            {panel === "tasks" && sessionId && (
              <BackgroundTasksPanel sessionId={sessionId} onClose={() => setPanel(null)} />
            )}
          </div>
        </header>
        {!desktop() && (
          <div className="host-banner">
            <Terminal size={16} />
            当前为界面预览，需要桌面宿主才能访问工作区和模型。
          </div>
        )}
        {error && (
          <div role="alert" className="alert">
            <span>{error}</span>
            <button aria-label="关闭错误" onClick={() => setError("")}>
              <X size={16} />
            </button>
          </div>
        )}
        {notice && (
          <div role="status" className="notice">
            <Check size={15} />
            {notice}
            <button aria-label="关闭提示" onClick={() => setNotice("")}>
              <X size={15} />
            </button>
          </div>
        )}
        <div
          className={`chat-body${navState.id === sessionId && navState.entries.length ? " has-turn-nav" : ""}`}
        >
          {navState.id === sessionId && (
            <TurnNavigator
              entries={navState.entries}
              activeId={navActive}
              onJump={(id) => void jumpToTurn(id)}
              onLoadOlder={() => void loadOlderNav()}
              hasOlder={navState.hasOlder}
              loading={navLoading}
            />
          )}
          {navError && (
            <button className="turn-nav-retry" onClick={retryNav}>
              重试加载回合导航
            </button>
          )}
          {jumping && (
            <button className="turn-nav-cancel" onClick={cancelJump}>
              取消跳转
            </button>
          )}
          <section
            ref={conversationRef}
            className="conversation"
            aria-label="聊天消息"
            onScroll={(event) => {
              const node = event.currentTarget;
              syncNavPosition(node);
              const atBottom = node.scrollHeight - node.scrollTop - node.clientHeight < 80;
              autoFollow.current = atBottom;
              setShowJumpBottom(!atBottom);
              if (node.scrollTop <= 8) void loadOlderTimeline();
            }}
          >
            {!timelineLoading && !events.length && !stream && (
              <div className="welcome">
                <span className="welcome-icon">
                  <Terminal size={35} />
                </span>
                <div className="eyebrow">BUILD WITH CONFIDENCE</div>
                <h1>
                  {drafting && draftWorkspaceId !== null ? (
                    <>
                      你想在 <em className="pj-name">{projectName(draftWorkspaceId)}</em>{" "}
                      中构建什么？
                    </>
                  ) : (
                    "想把什么变成现实？"
                  )}
                </h1>
                <p>
                  理解代码、探索想法，或完成下一次改进。
                  <br />
                  在独立的工作副本中协作，每一处更改都由你掌控。
                </p>
                {!workspaceId && !drafting && !sessionId && (
                  <button
                    className="primary welcome-open"
                    onClick={openWorkspace}
                    disabled={busy || !desktop()}
                  >
                    <FolderOpen size={15} />
                    打开工作区
                  </button>
                )}
                <div className="suggestions">
                  {[
                    "梳理项目结构和关键模块",
                    "检查代码，找出值得改进的地方",
                    "帮我规划并实现一个新功能",
                  ].map((s, i) => (
                    <button
                      key={s}
                      onClick={() => setComposerText(s)}
                      disabled={!sessionId && !drafting}
                    >
                      <span>0{i + 1}</span>
                      {s}
                      <ArrowUp size={14} />
                    </button>
                  ))}
                </div>
                <div className="welcome-foot">
                  <ShieldCheck size={15} />
                  工作副本隔离 · 操作可审批 · 更改可预览
                </div>
              </div>
            )}
            <div
              className="messages"
              onContextMenu={(event) => {
                event.preventDefault();
                setMsgContextMenu({
                  x: event.clientX,
                  y: event.clientY,
                  text: window.getSelection()?.toString().trim() ?? "",
                });
              }}
            >
              {timeline?.hasOlder ? (
                <div className="timeline-page-control">
                  <button
                    type="button"
                    onClick={() => void loadOlderTimeline()}
                    disabled={olderTimelineLoading}
                  >
                    {olderTimelineLoading ? "正在加载更早消息…" : "加载更早消息"}
                  </button>
                </div>
              ) : timeline && timeline.loadedOlder ? (
                <div className="timeline-page-control">
                  <small>已到会话开头</small>
                </div>
              ) : null}
              <ConversationEvents
                turns={timeline?.turns ?? []}
                events={injectModelNotes(
                  events,
                  modelNotes.filter((n) => n.sessionId === sessionId),
                )}
                workspaceId={workspaceId}
                onOpenSession={openSession}
                onOpenSideChat={(id) => openSideChat(id, sessionId)}
                activeIds={activeIds}
                askAnswers={askAnswers}
              />
              {thinkingVisible && (
                <div
                  className="message assistant thinking-indicator"
                  role="status"
                  aria-label="正在思考"
                >
                  <div className="message-body">
                    <div className="thinking-line">
                      <span>正在思考</span>
                      <span className="thinking-dots" aria-hidden="true">
                        <i />
                        <i />
                        <i />
                      </span>
                    </div>
                  </div>
                </div>
              )}
              {stream && (
                <article className="message assistant">
                  <div className="message-body">
                    <div className="stream-tag">正在思考与执行</div>
                    {stream.reasoning && (
                      <details className="reasoning">
                        <summary>思考过程</summary>
                        <pre>{stream.reasoning}</pre>
                      </details>
                    )}
                    <div className="prose">
                      <Markdown content={stream.text || "正在处理…"} />
                      <span className="cursor" />
                    </div>
                  </div>
                </article>
              )}
              <div ref={end} />
            </div>
          </section>
          {showJumpBottom && (
            <button
              type="button"
              className="jump-bottom"
              aria-label="滚动到底部"
              title="滚动到底部"
              onClick={jumpToBottom}
            >
              <ArrowDown size={16} />
            </button>
          )}
        </div>
        <footer className="composer-area">
          {drafting && (
            <div className="chip-row">
              <button
                type="button"
                className="chip"
                aria-haspopup="menu"
                aria-expanded={pjMenuOpen}
                aria-label="选择项目"
                onClick={() => setPjMenuOpen((v) => !v)}
              >
                <Folder size={13} aria-hidden="true" />
                <span className="nm">
                  {draftWorkspaceId !== null ? projectName(draftWorkspaceId) : "选择项目"}
                </span>
                <ChevronDown size={13} aria-hidden="true" />
              </button>
              {draftWorkspaceId && (
                <div className="mode-select">
                  <select
                    aria-label="执行位置"
                    value={draftExecMode}
                    onChange={(e) => setDraftExecOverride(e.target.value as ExecutionMode)}
                  >
                    <option value="direct">直接写入项目</option>
                    {workspaces.find((w) => w.id === draftWorkspaceId)?.executionMode ===
                      "replica" && <option value="replica">隔离副本（审阅后同步）</option>}
                    <option value="worktree">独立 worktree（需 git）</option>
                  </select>
                </div>
              )}
              {pjMenuOpen && (
                <div className="pj-menu" role="menu" aria-label="选择项目">
                  <div className="pj-search">
                    <Search size={13} aria-hidden="true" />
                    <input
                      value={pjQuery}
                      onChange={(e) => setPjQuery(e.target.value)}
                      placeholder="搜索项目"
                      aria-label="搜索项目"
                    />
                  </div>
                  <button
                    type="button"
                    role="menuitem"
                    className={draftWorkspaceId === "" ? "current" : ""}
                    onClick={() => {
                      setDraftWorkspaceId("");
                      setDraftExecOverride(null);
                      setPjMenuOpen(false);
                    }}
                  >
                    <MessageSquare size={13} aria-hidden="true" />
                    <span className="nm">无项目</span>
                    <span className="pj-sub">不关联目录的独立会话</span>
                    {draftWorkspaceId === "" && <Check size={14} aria-hidden="true" />}
                  </button>
                  <div className="pj-list">
                    {pjCandidates.map((ws) => (
                      <button
                        type="button"
                        key={ws.id}
                        role="menuitem"
                        className={ws.id === draftWorkspaceId ? "current" : ""}
                        onClick={() => {
                          setDraftWorkspaceId(ws.id);
                          setDraftExecOverride(null);
                          setPjMenuOpen(false);
                        }}
                      >
                        <Folder size={13} aria-hidden="true" />
                        <span className="nm">{ws.name}</span>
                        {ws.pinned && <Pin size={11} aria-hidden="true" />}
                        {ws.id === draftWorkspaceId && <Check size={14} aria-hidden="true" />}
                      </button>
                    ))}
                    {!pjCandidates.length && <p className="pj-empty">没有匹配的项目</p>}
                  </div>
                  <div className="pj-sep" />
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setPjMenuOpen(false);
                      void openWorkspace();
                    }}
                  >
                    <Plus size={13} aria-hidden="true" />
                    <span className="nm">新建项目</span>
                    <span className="pj-sub">打开文件夹</span>
                  </button>
                </div>
              )}
            </div>
          )}
          {quotes.length > 0 && (
            <div className="quote-row">
              {quotes.map((quote, index) => (
                <span key={`${index}-${quote.slice(0, 12)}`} className="quote-chip" title={quote}>
                  <Quote size={11} className="q-ico" aria-hidden="true" />
                  <span className="q-txt">{quote}</span>
                  <button
                    type="button"
                    className="q-x"
                    aria-label={`删除引用 ${index + 1}`}
                    onClick={() => setQuotes((old) => old.filter((_, i) => i !== index))}
                  >
                    <X size={9} />
                  </button>
                </span>
              ))}
            </div>
          )}
          {currentQuestion && (
            <QuestionCard
              key={currentQuestion.id}
              question={currentQuestion}
              sessionTitle={
                sessions.find((s) => s.id === currentQuestion.sessionId)?.title ??
                currentQuestion.sessionId
              }
              onSubmit={(answer) => void resolveQuestion(currentQuestion.id, answer)}
            />
          )}
          <form className="composer" onSubmit={submit}>
            <ComposerInput
              {...{
                activeSkill,
                activeSlash,
                slashIndex,
                selectedOptionRef,
                applySkillCompletion,
                executeSlash,
                textareaRef,
              }}
              {...{
                sessionCreating,
                sessionId,
                drafting,
                draftWorkspaceId,
                projectName,
                composerText,
                setComposerText,
              }}
              {...{ setCursor, setSlashIndex, applyCompletion, setDismissedToken }}
            />
            {attachments.length > 0 && (
              <div className="permission-tags">
                {attachments.map((a) => (
                  <span key={a.hash}>
                    {a.name}{" "}
                    <button
                      type="button"
                      aria-label={`移除附件 ${a.name}`}
                      onClick={() =>
                        setAttachments((old) => old.filter((item) => item.hash !== a.hash))
                      }
                    >
                      <X size={12} />
                    </button>
                  </span>
                ))}
              </div>
            )}
            <div className="composer-bottom">
              <div className="composer-plus-wrap">
                <button
                  type="button"
                  className="composer-plus"
                  aria-label="添加内容"
                  aria-haspopup="menu"
                  aria-expanded={plusOpen}
                  disabled={(!sessionId && !drafting) || sessionCreating}
                  onClick={() => setPlusOpen((v) => !v)}
                >
                  <Plus size={17} />
                </button>
                {plusOpen && (
                  <div className="plus-menu" role="menu" aria-label="添加内容">
                    <button
                      type="button"
                      role="menuitem"
                      disabled={!sessionId || importing || attachments.length >= 20}
                      onClick={() => {
                        setPlusOpen(false);
                        void addAttachment();
                      }}
                    >
                      <Paperclip size={15} />
                      <span>
                        <strong>附件</strong>
                        <small>选择本地文件，随消息发送（最多 20 MiB）</small>
                      </span>
                    </button>
                    <button
                      type="button"
                      role="menuitem"
                      disabled={
                        !!sessionId &&
                        (planState.id !== sessionId || planState.enabled === null || planState.busy)
                      }
                      onClick={() => {
                        setPlusOpen(false);
                        if (!sessionId) {
                          setDraftPlan(true);
                          setNotice("草稿将在发送时启用计划模式。");
                        } else if (
                          planState.id === sessionId &&
                          planState.enabled === false &&
                          !planState.busy
                        )
                          void setPlanMode(sessionId, true);
                      }}
                    >
                      <ListTodo size={15} />
                      <span>
                        <strong>进入计划模式</strong>
                        <small>为当前会话启用计划模式</small>
                      </span>
                    </button>
                    <button type="button" role="menuitem" onClick={() => insertPlusCommand("goal")}>
                      <Target size={15} />
                      <span>
                        <strong>目标</strong>
                        <small>插入 /goal，为本会话设定一个目标</small>
                      </span>
                    </button>
                  </div>
                )}
              </div>
              {goalPending && (
                <button
                  type="button"
                  className="composer-goal"
                  aria-label="目标模式"
                  title="目标模式：下一条发送的消息将作为本会话目标（点击取消）"
                  onClick={() => {
                    setGoalPending(false);
                    setNotice("");
                  }}
                >
                  <Goal size={17} />
                </button>
              )}
              {sessionId && (
                <div className="composer-jobs-wrap">
                  <button
                    type="button"
                    className="composer-jobs"
                    aria-label="运行中后台任务"
                    title="运行中后台任务：后台 Shell 与已运行时长"
                    aria-haspopup="dialog"
                    aria-controls="jobs-popover"
                    aria-expanded={jobsOpen}
                    onClick={() => setJobsOpen((v) => !v)}
                  >
                    <Terminal size={17} />
                    {taskCount > 0 && (
                      <span className="subagent-count">{taskCount > 9 ? "9+" : taskCount}</span>
                    )}
                  </button>
                  {jobsOpen && (
                    <ComposerJobsPanel sessionId={sessionId} onClose={() => setJobsOpen(false)} />
                  )}
                </div>
              )}
              <div className="composer-subagent-wrap">
                <button
                  type="button"
                  className="composer-subagent"
                  aria-label="子代理"
                  title="子代理"
                  aria-haspopup="dialog"
                  aria-controls="subagent-popover"
                  aria-expanded={subagentOpen}
                  disabled={!panelWorkspaceId}
                  onClick={() => setSubagentOpen((v) => !v)}
                >
                  <Bot size={17} />
                  {runningSubagentCount > 0 && (
                    <span className="subagent-count">
                      {runningSubagentCount > 9 ? "9+" : runningSubagentCount}
                    </span>
                  )}
                </button>
                {subagentOpen && (
                  <SubagentPanel
                    sessions={subagentSessions}
                    activeIds={activeIds}
                    sideIds={sideSessionIds}
                    onOpen={(s) => {
                      if (sideSessionIds.has(s.id)) {
                        openSideChat(s.id, sessionId);
                        setSubagentOpen(false);
                        return;
                      }
                      void openFromSidebar(s);
                    }}
                    onCancel={(id) => void cancelSubagent(id)}
                    onClose={() => setSubagentOpen(false)}
                  />
                )}
              </div>
              <div className="model-select">
                <button
                  type="button"
                  className="plan-mode-chip"
                  aria-label="计划模式"
                  aria-pressed={
                    sessionId && planState.id === sessionId && planState.enabled !== null
                      ? planState.enabled
                      : undefined
                  }
                  disabled={
                    sessionCreating ||
                    !!running[sessionId] ||
                    (!sessionId && !drafting) ||
                    (!!sessionId &&
                      (planState.id !== sessionId || planState.enabled === null || planState.busy))
                  }
                  onClick={() => void togglePlanMode()}
                >
                  {!sessionId
                    ? draftPlan
                      ? "计划模式（草稿待启用）"
                      : "计划模式（草稿）"
                    : planState.id !== sessionId || planState.enabled === null
                      ? "计划模式（未知）"
                      : planState.enabled
                        ? "计划模式：已开启"
                        : "计划模式：已关闭"}
                </button>
                <div className="mode-select">
                  <select
                    aria-label="权限模式"
                    value={sessionId ? approvalModeForSession(sessionId) : approvalMode}
                    onChange={(e) => chooseApprovalMode(e.target.value as ApprovalPolicy)}
                  >
                    {APPROVAL_MODES.map((m) => (
                      <option key={m.value} value={m.value} title={m.description}>
                        {m.label}
                      </option>
                    ))}
                  </select>
                </div>
                <ModelSelect
                  models={models}
                  providers={providers}
                  modelId={modelId}
                  effort={effort}
                  open={modelMenuOpen}
                  onOpenChange={setModelMenuOpen}
                  onPickModel={chooseModel}
                  onPickEffort={chooseEffort}
                  onOpenManage={() => openSettings("models")}
                />
              </div>
              <UsageRing
                models={models}
                providers={providers}
                modelId={modelId}
                sessionId={sessionId}
              />
              <div className="send-controls">
                {isRunning && !currentQuestion && (
                  <button type="button" className="send" aria-label="取消生成" onClick={cancel}>
                    <Square size={16} />
                  </button>
                )}
                <button
                  className="send"
                  type="submit"
                  aria-label={canResume ? "继续任务" : "发送消息"}
                  title={canResume ? "继续任务" : undefined}
                  disabled={
                    sessionCreating ||
                    importing ||
                    planState.busy ||
                    (!canResume && !composerText.trim() && !attachments.length && !quotes.length) ||
                    (!sessionId && (!drafting || draftWorkspaceId === null)) ||
                    (!modelId && !/^\/plan\s*$/.test(composerText.trim()))
                  }
                >
                  {canResume ? <Send size={19} /> : <ArrowUp size={19} />}
                </button>
              </div>
            </div>
          </form>
        </footer>
      </main>
      <SidePanel
        workspaceId={workspaceId}
        executionMode={sessionExecMode(session)}
        currentSessionId={sessionId}
        planRevision={planPanelRevision}
        onOpenSession={(id, workspace) => {
          useSidePanel.getState().close();
          openFromPanel(id, workspace);
        }}
        sideChat={{
          workspaceId,
          modelId,
          effort,
          sessions,
          activeIds,
          approvals,
          questions,
          onResolveApproval: (id, allow) => void resolve(id, allow),
          onResolveQuestion: (id, answer) => void resolveQuestion(id, answer),
          onOpenSession: (id) => void openSession(id),
        }}
      />
      {wsContextMenu && (
        <div
          className="session-context-menu"
          role="menu"
          aria-label="项目操作"
          style={{ left: wsContextMenu.x, top: wsContextMenu.y }}
          onContextMenu={(event) => event.preventDefault()}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const ws = workspaces.find((candidate) => candidate.id === wsContextMenu.workspaceId);
              if (ws)
                void command("workspace_open_dir", { workspaceId: ws.id }).catch((err) =>
                  setError(errorText(err)),
                );
              setWsContextMenu(null);
            }}
          >
            <FolderOpen size={14} />
            打开项目目录
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const ws = workspaces.find((candidate) => candidate.id === wsContextMenu.workspaceId);
              if (ws)
                setWsDialog({
                  mode: "edit",
                  id: ws.id,
                  name: ws.name,
                  execMode: ws.executionMode === "worktree" ? "direct" : ws.executionMode,
                  currentMode: ws.executionMode,
                });
              setWsContextMenu(null);
            }}
          >
            <Pencil size={14} />
            编辑
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const ws = workspaces.find((candidate) => candidate.id === wsContextMenu.workspaceId);
              if (ws) void togglePinned(ws);
              setWsContextMenu(null);
            }}
          >
            <Pin size={14} />
            {workspaces.find((candidate) => candidate.id === wsContextMenu.workspaceId)?.pinned
              ? "取消置顶"
              : "置顶"}
          </button>
          {duplicateSourcePaths.has(
            workspaces.find((candidate) => candidate.id === wsContextMenu.workspaceId)
              ?.sourcePath ?? "",
          ) && (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                const ws = workspaces.find(
                  (candidate) => candidate.id === wsContextMenu.workspaceId,
                );
                if (ws) openMergeConfirm(ws);
                setWsContextMenu(null);
              }}
            >
              <GitCompareArrows size={14} />
              合并同源项目…
            </button>
          )}
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const ws = workspaces.find((candidate) => candidate.id === wsContextMenu.workspaceId);
              if (ws) openRemoveConfirm(ws);
              setWsContextMenu(null);
            }}
          >
            <Trash2 size={14} />
            移除项目
          </button>
        </div>
      )}
      {wsRemove && (
        <Modal title={`移除项目「${wsRemove.name}」`} onClose={() => setWsRemove(null)}>
          <p className="muted">
            项目入口将从任务列表隐藏；磁盘上的原项目、会话与自动化数据全部保留，不受影响。
          </p>
          <p className="muted">重新打开同一目录的项目时会重新显示它。</p>
          {wsRemove.running && (
            <p role="alert" className="inline-error">
              该项目还有运行中的对话或 Agent，移除会停止并释放相关运行状态。
            </p>
          )}
          <div className="actions">
            <button type="button" onClick={() => setWsRemove(null)}>
              取消
            </button>
            <button className="danger" type="button" onClick={() => void removeWorkspace()}>
              移除项目{wsRemove.running ? "（移除并停止运行）" : ""}
            </button>
          </div>
        </Modal>
      )}
      {wsMerge && (
        <Modal title="合并同源项目" onClose={() => setWsMerge(null)}>
          <p className="muted">
            同一源目录存在多条项目记录（常见于数据目录迁移后重新打开项目）。合并会把重复项目的会话（含归档）、自动化与定时任务挂到保留项目，并删除重复项目的记录与隔离副本。
          </p>
          <p className="muted">
            保留项目：<strong>{projectName(wsMergeKeepId)}</strong>
            （自动选择创建时间最早的项目，其余同源项目全部并入）。
          </p>
          <p className="muted">
            待并入并删除记录的项目：
            {workspaces
              .filter((ws) => ws.sourcePath === wsMerge.sourcePath && ws.id !== wsMergeKeepId)
              .map((ws) => ws.name)
              .join("、") || "（无）"}
            ；副本中未同步的改动将随之删除，不可恢复。
          </p>
          {workspaces.some(
            (ws) =>
              ws.sourcePath === wsMerge.sourcePath &&
              ws.id !== wsMergeKeepId &&
              ws.executionMode === "worktree",
          ) && (
            <p className="muted">
              worktree 项目在源项目下代建的会话 worktree 目录不会自动删除，合并后请手动清理。
            </p>
          )}
          {workspaces.some(
            (ws) =>
              ws.sourcePath === wsMerge.sourcePath &&
              ws.id !== wsMergeKeepId &&
              allSessions.some(
                (s) => s.workspaceId === ws.id && (running[s.id] || activeIds.has(s.id)),
              ),
          ) && (
            <p role="alert" className="inline-error">
              待并入项目还有运行中的对话或 Agent，合并会停止并释放相关运行状态。
            </p>
          )}
          <div className="actions">
            <button type="button" onClick={() => setWsMerge(null)}>
              取消
            </button>
            <button className="danger" type="button" onClick={() => void mergeWorkspace()}>
              合并
            </button>
          </div>
        </Modal>
      )}
      {sessionContextMenu && (
        <div
          className="session-context-menu"
          role="menu"
          aria-label="会话操作"
          style={{ left: sessionContextMenu.x, top: sessionContextMenu.y }}
          onContextMenu={(event) => event.preventDefault()}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const item = allSessions.find(
                (candidate) => candidate.id === sessionContextMenu.sessionId,
              );
              if (item) openSessionRename(item);
              setSessionContextMenu(null);
            }}
          >
            <Pencil size={14} />
            重命名
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const item = allSessions.find(
                (candidate) => candidate.id === sessionContextMenu.sessionId,
              );
              if (item) archiveSession(item);
              setSessionContextMenu(null);
            }}
          >
            <Archive size={14} />
            存档
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              exportSession(sessionContextMenu.sessionId);
              setSessionContextMenu(null);
            }}
          >
            <Download size={14} />
            导出会话
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              void navigator.clipboard?.writeText(sessionContextMenu.sessionId);
              setSessionContextMenu(null);
            }}
          >
            <Copy size={14} />
            复制会话ID
          </button>
        </div>
      )}
      {msgContextMenu && (
        <div
          className="session-context-menu"
          role="menu"
          aria-label="消息操作"
          style={{ left: msgContextMenu.x, top: msgContextMenu.y }}
          onContextMenu={(event) => event.preventDefault()}
        >
          <button
            type="button"
            role="menuitem"
            disabled={!msgContextMenu.text}
            onClick={() => {
              if (msgContextMenu.text)
                setQuotes((old) => [...old, msgContextMenu.text].slice(0, 10));
              setMsgContextMenu(null);
              window.getSelection()?.removeAllRanges();
            }}
          >
            <Quote size={14} />
            引用
          </button>
          <button
            type="button"
            role="menuitem"
            disabled={!msgContextMenu.text}
            onClick={() => {
              if (msgContextMenu.text) void navigator.clipboard?.writeText(msgContextMenu.text);
              setMsgContextMenu(null);
            }}
          >
            <Copy size={14} />
            复制
          </button>
        </div>
      )}
      {sessionRename && (
        <Modal title="重命名会话" onClose={closeSessionRename}>
          <form
            className="settings-form"
            onSubmit={(event) => {
              event.preventDefault();
              void saveSessionRename();
            }}
          >
            <label>
              会话名称
              <input
                autoFocus
                disabled={sessionRenameSavingIds.has(sessionRename.id)}
                value={sessionRename.name}
                onChange={(event) => {
                  setSessionRename((old) => old && { ...old, name: event.target.value });
                  setSessionRenameError("");
                }}
              />
            </label>
            {sessionRenameError && (
              <p role="alert" className="inline-error">
                {sessionRenameError}
              </p>
            )}
            <div className="actions">
              <button type="button" onClick={closeSessionRename}>
                取消
              </button>
              <button
                className="primary"
                type="submit"
                disabled={sessionRenameSavingIds.has(sessionRename.id)}
              >
                保存
              </button>
            </div>
          </form>
        </Modal>
      )}
      {panel === "presets" && (
        <PresetsPanel models={models} providers={providers} onClose={() => setPanel(null)} />
      )}
      {panel === "skills" && (
        <SkillsPanel workspaceId={workspaceId} onClose={() => setPanel(null)} />
      )}
      <PluginThemeHost workspaceId={workspaceId} revision={pluginRevision} />
      {panel === "plugins" && (
        <PluginPanel
          workspaceId={workspaceId}
          sessionId={sessionId}
          revision={pluginRevision}
          onChanged={() => setPluginRevision((v) => v + 1)}
          onClose={() => setPanel(null)}
        />
      )}

      {panel === "automations" && (
        <Modal title="自动化任务" onClose={() => setPanel(null)}>
          <AutomationsPanel
            workspaceId={workspaceId}
            workspaces={workspaces}
            allSessions={allSessions}
            onChanged={() => {
              if (desktop())
                command<unknown[]>("automation_list", { workspaceId })
                  .then((list) => setAutomationCount(list.length))
                  .catch(() => {});
            }}
            onOpenSession={openFromPanel}
          />
        </Modal>
      )}
      {wsDialog && (
        <Modal
          title={wsDialog.mode === "create" ? "打开工作区" : "编辑项目"}
          onClose={() => setWsDialog(null)}
        >
          <form
            className="settings-form"
            onSubmit={(e) => {
              e.preventDefault();
              void (wsDialog.mode === "create" ? createWorkspace() : saveWorkspaceEdit());
            }}
          >
            {wsDialog.mode === "create" && <p className="muted">{wsDialog.path}</p>}
            <label>
              项目名称
              <input
                value={wsDialog.name}
                onChange={(e) => setWsDialog((old) => old && { ...old, name: e.target.value })}
                placeholder="默认使用目录名"
              />
            </label>
            {wsDialog.mode === "create" && (
              <p className="muted">留空则使用目录名「{basename(wsDialog.path)}」。</p>
            )}
            <label>
              默认执行位置
              <select
                value={wsDialog.execMode}
                onChange={(e) =>
                  setWsDialog((old) =>
                    old ? { ...old, execMode: e.target.value as ExecutionMode } : old,
                  )
                }
              >
                <option value="direct">直接写入源项目：不复制、不建 worktree，改动立即生效</option>
                <option value="replica">隔离副本：改动经审阅确认后同步到原项目</option>
              </select>
            </label>
            {wsDialog.mode === "edit" && wsDialog.currentMode === "worktree" && (
              <p className="muted">
                该项目由旧版 worktree 模式创建。保存后新会话默认走所选执行位置；已创建的会话
                worktree 目录与分支保留，已登记 worktree 的旧会话继续在原 worktree 执行。
              </p>
            )}
            {wsDialog.mode === "edit" && wsDialog.execMode !== wsDialog.currentMode && (
              <p className="muted">
                切换从下一回合起对未固化执行位置的会话生效，运行中的会话需结束后才能切换。
                {wsDialog.execMode === "replica"
                  ? "若此前没有隔离副本，会从源项目重建；已创建的会话 worktree 目录与分支不会删除。"
                  : "隔离副本中未同步的改动会保留，切回副本后可继续审阅。"}
              </p>
            )}
            <div className="actions">
              <button type="button" onClick={() => setWsDialog(null)}>
                取消
              </button>
              <button className="primary" type="submit">
                {wsDialog.mode === "create" ? "创建" : "保存"}
              </button>
            </div>
          </form>
        </Modal>
      )}
      {mainApproval && (
        <Modal title="操作需要你的批准" onClose={() => void resolve(mainApproval.id, false)}>
          <ApprovalPrompt
            approval={mainApproval}
            sessionTitle={
              sessions.find((s) => s.id === mainApproval.sessionId)?.title ?? mainApproval.sessionId
            }
            onResolve={(allow) => void resolve(mainApproval.id, allow)}
          />
        </Modal>
      )}
      {settingsOpen &&
        createPortal(
          <SettingsPage
            key={settingsSection ?? "overview"}
            onClose={() => setSettingsOpen(false)}
            initialSection={settingsSection ?? undefined}
            onOpenPanel={(next) => {
              setPanel(next);
              setSettingsOpen(false);
            }}
            models={models}
            onModelsSaved={refreshModels}
            archivedSessions={archivedSessions}
            projectName={projectName}
            onRestore={restoreSession}
          />,
          document.body,
        )}
    </div>
  );
}
const EMPTY_EVENTS: SessionEvent[] = [];
// 模型切换标记（内存伪事件）：锚定切换时刻的 seq 注入时间线渲染；不落库、刷新即逝。
type ModelNote = { id: string; sessionId: string; text: string; anchorSeq: string };
function modelNoteEvent(note: ModelNote): SessionEvent {
  return {
    id: note.id,
    sessionId: note.sessionId,
    seq: note.anchorSeq,
    kind: "ui.model_note",
    data: { text: note.text },
    createdAt: "",
  };
}
// 标记插到锚点（切换时最后事件）之后、第一个更大 seq 的事件之前；锚点晚于全部事件（如历史页未加载）时附加末尾。
function injectModelNotes(events: SessionEvent[], notes: ModelNote[]): SessionEvent[] {
  if (!notes.length) return events;
  const ordered = [...notes].sort((a, b) => (BigInt(a.anchorSeq) < BigInt(b.anchorSeq) ? -1 : 1));
  const out: SessionEvent[] = [];
  let next = 0;
  for (const event of events) {
    while (next < ordered.length && BigInt(ordered[next].anchorSeq) < BigInt(event.seq))
      out.push(modelNoteEvent(ordered[next++]));
    out.push(event);
  }
  while (next < ordered.length) out.push(modelNoteEvent(ordered[next++]));
  return out;
}
