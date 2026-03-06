import { useState } from 'react';
import {
  AlertTriangle,
  CheckCircle,
  Info,
  Loader2,
  Save,
  Wand2,
  X,
  Zap,
} from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { Badge } from '../ui/Badge';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import type { McpServer, McpToolInfo, SaveMcpServerInput } from '../../types/extensions';

interface McpServerFormProps {
  server?: McpServer;
  onSave: (input: SaveMcpServerInput) => void;
  onCancel: () => void;
}

type TransportType = 'stdio' | 'sse' | 'streamable_http';

function parseArgsDraft(
  raw: string,
  messages: {
    invalidJson: string;
    invalidArray: string;
  },
): { values: string[]; error: string | null } {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { values: [], error: null };
  }

  if (trimmed.startsWith('[')) {
    try {
      const parsed = JSON.parse(trimmed);
      if (!Array.isArray(parsed) || !parsed.every((value) => typeof value === 'string')) {
        return { values: [], error: messages.invalidArray };
      }
      return { values: parsed, error: null };
    } catch {
      return { values: [], error: messages.invalidJson };
    }
  }

  const values = (
    trimmed.includes('\n')
      ? trimmed.split('\n')
      : trimmed.includes(',')
        ? trimmed.split(',')
        : trimmed.split(/\s+/)
  )
    .map((value) => value.trim())
    .filter(Boolean);

  return { values, error: null };
}

function parseJsonObjectDraft(
  raw: string,
  label: string,
  messages: {
    mustBeObject: string;
    invalidEntries: string;
    invalidJson: string;
  },
): { normalized: string | null; error: string | null } {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { normalized: null, error: null };
  }

  try {
    const parsed = JSON.parse(trimmed);
    if (parsed === null || Array.isArray(parsed) || typeof parsed !== 'object') {
      return { normalized: null, error: `${label}${messages.mustBeObject}` };
    }

    const entries = Object.entries(parsed as Record<string, unknown>);
    if (!entries.every(([key, value]) => key.trim().length > 0 && typeof value === 'string')) {
      return { normalized: null, error: `${label}${messages.invalidEntries}` };
    }

    return { normalized: JSON.stringify(parsed), error: null };
  } catch {
    return { normalized: null, error: `${label}${messages.invalidJson}` };
  }
}

function normalizeHttpUrl(raw: string): string {
  const value = raw.trim();
  if (!value) {
    return '';
  }

  const parsed = new URL(value);
  if (!['http:', 'https:'].includes(parsed.protocol)) {
    throw new Error('protocol');
  }
  return parsed.toString();
}

export function McpServerForm({ server, onSave, onCancel }: McpServerFormProps) {
  const { t, locale } = useTranslation();
  const isChinese = locale.startsWith('zh');
  const copy = {
    transportLabel: isChinese ? 'Transport' : 'Transport',
    transportHelp: isChinese
      ? '前端只显示当前后端真正支持的 transport，避免保存后才发现无法使用。'
      : 'Only transports that are implemented end-to-end are shown here.',
    stdioTitle: isChinese ? 'stdio' : 'stdio',
    stdioHint: isChinese
      ? '本地子进程 MCP。适合 `npx`、`uvx`、`docker run` 等命令。'
      : 'Launch a local MCP process such as `npx`, `uvx`, or `docker run`.',
    streamableTitle: isChinese ? 'Streamable HTTP' : 'Streamable HTTP',
    streamableHint: isChinese
      ? '推荐。单一 HTTP endpoint，支持会话恢复与 JSON/SSE 响应。'
      : 'Recommended. Single HTTP endpoint with session-aware JSON or SSE responses.',
    sseTitle: isChinese ? 'SSE (legacy)' : 'SSE (legacy)',
    sseHint: isChinese
      ? '兼容旧版远程 MCP：先连 SSE，再按 endpoint 事件回传的地址 POST 消息。'
      : 'Compatibility mode for older remote MCP servers that expose an SSE stream plus a message endpoint.',
    recommended: isChinese ? '推荐' : 'Recommended',
    nameRequired: isChinese ? '请输入服务器名称。' : 'Enter a server name.',
    commandRequired: isChinese ? '请输入启动命令。' : 'Enter a launch command.',
    urlRequired: isChinese ? '请输入 URL。' : 'Enter a URL.',
    urlInvalid: isChinese ? 'URL 必须是 http 或 https。' : 'URL must use http or https.',
    argsHelp: isChinese
      ? '推荐直接填写 JSON 数组；也支持每行一个参数、逗号分隔或空格分隔。'
      : 'Recommended: use a JSON array. One-arg-per-line, comma-separated, and whitespace-separated input also work.',
    envHelp: isChinese
      ? '使用 JSON 对象。若未显式设置 `PORT`，应用会自动注入 `PORT=0`，让系统分配空闲端口。'
      : 'Use a JSON object. If `PORT` is omitted, the app injects `PORT=0` so the OS can assign a free port.',
    urlHelp: isChinese
      ? '请输入完整 MCP endpoint。`streamable_http` 通常类似 `https://host/mcp`。'
      : 'Enter the full MCP endpoint URL. `streamable_http` is typically something like `https://host/mcp`.',
    headersHelp: isChinese
      ? '使用 JSON 对象，例如 `{\"Authorization\":\"Bearer ...\"}`。'
      : 'Use a JSON object, for example `{\"Authorization\":\"Bearer ...\"}`.',
    argsInvalidJson: isChinese ? '参数 JSON 格式无效。' : 'Arguments JSON is invalid.',
    argsInvalidArray: isChinese ? '参数 JSON 必须是字符串数组。' : 'Arguments JSON must be an array of strings.',
    jsonMustBeObject: isChinese ? '必须是 JSON 对象。' : ' must be a JSON object.',
    jsonInvalidEntries: isChinese
      ? '的 key 不能为空，value 必须是字符串。'
      : ' keys must be non-empty and values must be strings.',
    jsonInvalid: isChinese ? 'JSON 格式无效。' : ' JSON is invalid.',
    parsedArgs: isChinese ? '解析后的参数' : 'Parsed arguments',
    draftReady: isChinese ? '当前配置可以直接测试和保存。' : 'This draft is ready to test and save.',
    draftInvalid: isChinese ? '请先修复高亮字段。' : 'Fix the highlighted fields before testing or saving.',
    freePortManaged: isChinese
      ? '若目标命令需要端口，应用会自动使用空闲端口，避免默认端口被占用时直接失败。'
      : 'If the command needs a port, the app can inject a free port automatically instead of failing on a busy default port.',
    explicitPort: isChinese
      ? '你已经显式设置了 `PORT`，应用不会覆盖它。'
      : 'You already set `PORT` explicitly, so the app will not override it.',
    remoteSessionNote: isChinese
      ? '远程 transport 会保留 headers 并处理 session id；`streamable_http` 在会话过期时会尝试重新初始化。'
      : 'Remote transports keep custom headers and handle session ids; `streamable_http` will attempt to reinitialize when a session expires.',
    urlLabel: isChinese ? 'URL' : 'URL',
    headersLabel: isChinese ? 'HTTP Headers' : 'HTTP Headers',
  };

  const initialTransport = (server?.transport ?? 'stdio') as TransportType;
  const [name, setName] = useState(server?.name ?? '');
  const [transport, setTransport] = useState<TransportType>(
    ['stdio', 'sse', 'streamable_http'].includes(initialTransport) ? initialTransport : 'stdio',
  );
  const [command, setCommand] = useState(server?.command ?? '');
  const [args, setArgs] = useState(server?.args ?? '');
  const [envJson, setEnvJson] = useState(server?.envJson ?? '');
  const [url, setUrl] = useState(server?.url ?? '');
  const [headersJson, setHeadersJson] = useState(server?.headersJson ?? '');
  const [testLoading, setTestLoading] = useState(false);
  const [discoveredTools, setDiscoveredTools] = useState<McpToolInfo[] | null>(null);
  const [testError, setTestError] = useState<string | null>(null);

  const isStdio = transport === 'stdio';
  const isRemote = !isStdio;

  const parsedArgs = parseArgsDraft(args, {
    invalidJson: copy.argsInvalidJson,
    invalidArray: copy.argsInvalidArray,
  });
  const parsedEnv = parseJsonObjectDraft(envJson, t('settings.mcpEnvVars'), {
    mustBeObject: copy.jsonMustBeObject,
    invalidEntries: copy.jsonInvalidEntries,
    invalidJson: copy.jsonInvalid,
  });
  const parsedHeaders = parseJsonObjectDraft(headersJson, copy.headersLabel, {
    mustBeObject: copy.jsonMustBeObject,
    invalidEntries: copy.jsonInvalidEntries,
    invalidJson: copy.jsonInvalid,
  });

  const normalizedName = name.trim();
  const normalizedCommand = command.trim();
  const normalizedUrl = url.trim();

  let normalizedRemoteUrl: string | null = null;
  let urlError: string | null = null;
  if (isRemote) {
    if (!normalizedUrl) {
      urlError = copy.urlRequired;
    } else {
      try {
        normalizedRemoteUrl = normalizeHttpUrl(normalizedUrl);
      } catch {
        urlError = copy.urlInvalid;
      }
    }
  }

  const nameError = normalizedName ? null : copy.nameRequired;
  const commandError = isStdio && !normalizedCommand ? copy.commandRequired : null;
  const validationErrors = [
    nameError,
    commandError,
    isStdio ? parsedArgs.error : null,
    isStdio ? parsedEnv.error : null,
    isRemote ? urlError : null,
    isRemote ? parsedHeaders.error : null,
  ].filter(Boolean);
  const hasValidationError = validationErrors.length > 0;

  const envPortState =
    isStdio && parsedEnv.normalized
      ? Object.keys(JSON.parse(parsedEnv.normalized) as Record<string, string>).some(
          (key) => key.toUpperCase() === 'PORT',
        )
        ? ('explicit' as const)
        : ('managed' as const)
      : ('managed' as const);

  const transportCards: Array<{
    value: TransportType;
    title: string;
    hint: string;
    badge?: string;
  }> = [
    { value: 'stdio', title: copy.stdioTitle, hint: copy.stdioHint },
    {
      value: 'streamable_http',
      title: copy.streamableTitle,
      hint: copy.streamableHint,
      badge: copy.recommended,
    },
    { value: 'sse', title: copy.sseTitle, hint: copy.sseHint },
  ];

  const buildInput = (): SaveMcpServerInput | null => {
    if (hasValidationError) {
      return null;
    }

    if (isStdio) {
      return {
        id: server?.id ?? null,
        name: normalizedName,
        transport,
        command: normalizedCommand,
        args: parsedArgs.values.length > 0 ? JSON.stringify(parsedArgs.values) : null,
        url: null,
        envJson: parsedEnv.normalized,
        headersJson: null,
        enabled: server?.enabled ?? true,
      };
    }

    return {
      id: server?.id ?? null,
      name: normalizedName,
      transport,
      command: null,
      args: null,
      url: normalizedRemoteUrl,
      envJson: null,
      headersJson: parsedHeaders.normalized,
      enabled: server?.enabled ?? true,
    };
  };

  const handleSubmit = () => {
    const input = buildInput();
    if (!input) {
      return;
    }
    onSave(input);
  };

  const handleTest = async () => {
    const input = buildInput();
    if (!input) {
      return;
    }

    setTestLoading(true);
    setTestError(null);
    setDiscoveredTools(null);

    try {
      const tools = await api.testMcpServerDirect({
        name: input.name,
        transport: input.transport,
        command: input.command ?? null,
        args: input.args ?? null,
        url: input.url ?? null,
        envJson: input.envJson ?? null,
        headersJson: input.headersJson ?? null,
      });
      setDiscoveredTools(tools);
      toast.success(t('settings.mcpTestSuccess', { count: String(tools.length) }));
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      setTestError(message);
      toast.error(`${t('settings.mcpTestFailed')}: ${message}`);
    } finally {
      setTestLoading(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="rounded-xl border border-border bg-surface-2 p-4 space-y-3">
        <div className="flex items-start gap-2">
          <Info size={16} className="mt-0.5 shrink-0 text-accent" />
          <div className="space-y-1">
            <p className="text-sm font-medium text-text-primary">{copy.transportLabel}</p>
            <p className="text-xs text-text-tertiary">{copy.transportHelp}</p>
          </div>
        </div>
        <div className="grid gap-2 md:grid-cols-3">
          {transportCards.map((item) => {
            const active = transport === item.value;
            return (
              <button
                key={item.value}
                type="button"
                onClick={() => setTransport(item.value)}
                className={`rounded-xl border p-3 text-left transition-colors ${
                  active
                    ? 'border-accent bg-accent/10'
                    : 'border-border bg-surface-1 hover:bg-surface-2'
                }`}
              >
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-text-primary">{item.title}</span>
                  {item.badge && (
                    <Badge variant="default" className="text-[10px] bg-accent/10 text-accent border-accent/20">
                      {item.badge}
                    </Badge>
                  )}
                </div>
                <p className="mt-1 text-xs text-text-tertiary">{item.hint}</p>
              </button>
            );
          })}
        </div>
      </div>

      <div className="space-y-2">
        <label className="text-sm font-medium text-text-primary">{t('settings.mcpServerName')}</label>
        <Input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder={t('settings.mcpServerName')}
          error={nameError ?? undefined}
        />
      </div>

      {isStdio ? (
        <>
          <div className="space-y-2">
            <label className="text-sm font-medium text-text-primary">{t('settings.mcpCommand')}</label>
            <Input
              value={command}
              onChange={(event) => setCommand(event.target.value)}
              placeholder="npx"
              error={commandError ?? undefined}
            />
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium text-text-primary">{t('settings.mcpArgs')}</label>
            <textarea
              value={args}
              onChange={(event) => setArgs(event.target.value)}
              placeholder='["-y", "@modelcontextprotocol/server-filesystem", "D:/vault"]'
              rows={4}
              className={`w-full rounded-md border bg-surface-2 px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 resize-y ${
                parsedArgs.error
                  ? 'border-danger focus:border-danger focus:ring-danger/30'
                  : 'border-border focus:border-accent focus:ring-accent'
              }`}
            />
            <p className="text-xs text-text-tertiary">{copy.argsHelp}</p>
            {parsedArgs.error && <p className="text-xs text-danger">{parsedArgs.error}</p>}
            {parsedArgs.values.length > 0 && (
              <div className="rounded-lg border border-border bg-surface-2 p-3 space-y-2">
                <p className="text-[11px] uppercase tracking-wide text-text-tertiary">{copy.parsedArgs}</p>
                <div className="flex flex-wrap gap-1.5">
                  {parsedArgs.values.map((value, index) => (
                    <Badge key={`${value}-${index}`} variant="default" className="max-w-full text-[10px]">
                      {value}
                    </Badge>
                  ))}
                </div>
              </div>
            )}
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium text-text-primary">{t('settings.mcpEnvVars')}</label>
            <textarea
              value={envJson}
              onChange={(event) => setEnvJson(event.target.value)}
              placeholder='{"API_KEY": "..."}'
              rows={4}
              className={`w-full rounded-md border bg-surface-2 px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 resize-y ${
                parsedEnv.error
                  ? 'border-danger focus:border-danger focus:ring-danger/30'
                  : 'border-border focus:border-accent focus:ring-accent'
              }`}
            />
            <p className="text-xs text-text-tertiary">{copy.envHelp}</p>
            {parsedEnv.error && <p className="text-xs text-danger">{parsedEnv.error}</p>}
          </div>
        </>
      ) : (
        <>
          <div className="space-y-2">
            <label className="text-sm font-medium text-text-primary">{copy.urlLabel}</label>
            <Input
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              placeholder={transport === 'sse' ? 'https://example.com/sse' : 'https://example.com/mcp'}
              error={urlError ?? undefined}
            />
            <p className="text-xs text-text-tertiary">{copy.urlHelp}</p>
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium text-text-primary">{copy.headersLabel}</label>
            <textarea
              value={headersJson}
              onChange={(event) => setHeadersJson(event.target.value)}
              placeholder='{"Authorization": "Bearer ..."}'
              rows={4}
              className={`w-full rounded-md border bg-surface-2 px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 resize-y ${
                parsedHeaders.error
                  ? 'border-danger focus:border-danger focus:ring-danger/30'
                  : 'border-border focus:border-accent focus:ring-accent'
              }`}
            />
            <p className="text-xs text-text-tertiary">{copy.headersHelp}</p>
            {parsedHeaders.error && <p className="text-xs text-danger">{parsedHeaders.error}</p>}
          </div>
        </>
      )}

      <div
        className={`rounded-xl border p-4 ${
          hasValidationError ? 'border-warning/30 bg-warning/10' : 'border-success/20 bg-success/10'
        }`}
      >
        <div className="flex items-start gap-2">
          {hasValidationError ? (
            <AlertTriangle size={16} className="mt-0.5 shrink-0 text-warning" />
          ) : (
            <CheckCircle size={16} className="mt-0.5 shrink-0 text-success" />
          )}
          <div className="space-y-1">
            <p className={`text-sm font-medium ${hasValidationError ? 'text-warning' : 'text-success'}`}>
              {hasValidationError ? copy.draftInvalid : copy.draftReady}
            </p>
            <p className="text-xs text-text-tertiary">
              {isStdio
                ? envPortState === 'managed'
                  ? copy.freePortManaged
                  : copy.explicitPort
                : copy.remoteSessionNote}
            </p>
          </div>
        </div>
      </div>

      <div className="space-y-2">
        <Button
          variant="secondary"
          size="sm"
          icon={testLoading ? <Loader2 size={14} className="animate-spin" /> : <Wand2 size={14} />}
          loading={testLoading}
          onClick={handleTest}
          disabled={hasValidationError}
        >
          {t('settings.mcpTestConnection')}
        </Button>
        {testError && (
          <div className="rounded-lg border border-danger/30 bg-danger/5 p-3">
            <p className="text-sm text-danger whitespace-pre-wrap break-all">{testError}</p>
          </div>
        )}
        {discoveredTools && (
          <div className="rounded-lg border border-border bg-surface-2 p-3 space-y-2">
            <div className="flex items-center gap-2">
              <Zap size={14} className="text-accent" />
              <p className="text-sm font-medium text-text-primary">
                {t('settings.mcpDiscoveredTools')} ({discoveredTools.length})
              </p>
            </div>
            <div className="flex flex-wrap gap-1.5">
              {discoveredTools.map((tool) => (
                <Badge key={tool.name} variant="default" className="text-[10px]">
                  {tool.name}
                </Badge>
              ))}
            </div>
          </div>
        )}
      </div>

      <div className="flex items-center gap-2 pt-2 border-t border-border">
        <Button
          variant="primary"
          size="sm"
          icon={<Save size={14} />}
          onClick={handleSubmit}
          disabled={hasValidationError}
        >
          {t('common.save')}
        </Button>
        <Button variant="ghost" size="sm" icon={<X size={14} />} onClick={onCancel}>
          {t('common.cancel')}
        </Button>
      </div>
    </div>
  );
}
