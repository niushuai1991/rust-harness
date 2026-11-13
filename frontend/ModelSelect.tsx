import { useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { Check, ChevronDown, RotateCcw, Search, Settings2, Zap } from 'lucide-react';
import type { ModelConfig, ProviderInfo } from './types';

export type ModelSelectProps = {
  models: ModelConfig[];
  providers: ProviderInfo[];
  modelId: string;
  effort: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPickModel: (id: string) => void;
  onPickEffort: (level: string) => void;
  onOpenManage: () => void;
};

type Row = { model: ModelConfig; provider: ProviderInfo };

// 浮层卡片选择器：模型分组列表 + 档位滑块（参考 ChatGPT 的选择器形态）。
// 卡片在切换模型后保持展开，供连续调整档位；Esc 或点击外部关闭。
export default function ModelSelect({
  models,
  providers,
  modelId,
  effort,
  open,
  onOpenChange,
  onPickModel,
  onPickEffort,
  onOpenManage,
}: ModelSelectProps) {
  const [query, setQuery] = useState('');
  const [highlight, setHighlight] = useState(0);
  const wrapRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const activeModel = models.find((m) => m.id === modelId);
  const efforts = activeModel?.efforts ?? [];
  const activeIndex =
    effort && efforts.includes(effort) ? efforts.indexOf(effort) : Math.max(0, efforts.length - 1);
  const fill = efforts.length > 1 ? activeIndex / (efforts.length - 1) : 0;

  // 分组：仅展示启用供应商；行内保留停用模型（灰显 + 原因），供了解目录全貌。
  const groups = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    return providers
      .filter((p) => p.enabled)
      .map((provider) => ({
        provider,
        rows: models
          .filter((m) => m.providerId === provider.id)
          .filter(
            (m) =>
              !keyword ||
              `${m.id} ${m.name} ${provider.name}`.toLowerCase().includes(keyword),
          ),
      }))
      .filter((group) => group.rows.length > 0);
  }, [models, providers, query]);

  const enabledRows = useMemo(
    () =>
      groups.flatMap((group) =>
        group.rows.filter((m) => m.enabled).map((model) => ({ model, provider: group.provider })),
      ),
    [groups],
  );

  useEffect(() => {
    setHighlight(0);
  }, [query, open]);

  useEffect(() => {
    if (!open) return;
    setQuery('');
    const timer = window.setTimeout(() => searchRef.current?.focus(), 0);
    const onDown = (event: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(event.target as Node)) {
        onOpenChange(false);
      }
    };
    document.addEventListener('mousedown', onDown);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener('mousedown', onDown);
    };
  }, [open, onOpenChange]);

  function pickRow(row: Row) {
    if (!row.model.enabled) return;
    if (row.model.id === modelId) {
      onOpenChange(false);
      return;
    }
    onPickModel(row.model.id);
  }

  function onSearchKeyDown(event: ReactKeyboardEvent) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (!enabledRows.length) return;
      setHighlight((old) =>
        event.key === 'ArrowDown'
          ? Math.min(old + 1, enabledRows.length - 1)
          : Math.max(old - 1, 0),
      );
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const row = enabledRows[highlight];
      if (row) pickRow(row);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      onOpenChange(false);
    }
  }

  const triggerLabel = activeModel ? activeModel.name || activeModel.model : '选择模型';
  const sliderFill = `linear-gradient(to right,var(--accent) calc(8px + (100% - 16px) * ${fill}),var(--border) calc(8px + (100% - 16px) * ${fill}))`;
  let flatIndex = -1;

  return (
    <div className="model-select-wrap" ref={wrapRef}>
      <button
        type="button"
        className="model-trigger"
        aria-label="当前模型"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
      >
        <span className="model-dot" />
        <span className="model-trigger-name">{triggerLabel}</span>
        {efforts.length > 0 && <span className="model-trigger-effort">{efforts[activeIndex]}</span>}
        <ChevronDown size={13} aria-hidden />
      </button>
      {open && (
        <div className="model-menu" role="listbox" aria-label="选择模型">
          <div className="mm-search">
            <Search size={13} aria-hidden />
            <input
              ref={searchRef}
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onSearchKeyDown}
              placeholder="搜索模型 / 供应商…"
              aria-label="搜索模型"
            />
          </div>
          <div className="mm-list">
            {groups.map((group) => (
              <div key={group.provider.id}>
                <div className="mm-group">
                  <span>{group.provider.name}</span>
                  <em>{group.provider.builtin ? '内置' : '自定义'}</em>
                </div>
                {group.rows.map((model) => {
                  const enabled = model.enabled;
                  if (enabled) flatIndex += 1;
                  const row = { model, provider: group.provider };
                  const highlighted = enabled && flatIndex === highlight;
                  return (
                    <button
                      type="button"
                      key={model.id}
                      className={`mm-row${model.id === modelId ? ' current' : ''}${enabled ? '' : ' disabled'}${highlighted ? ' sel' : ''}`}
                      onClick={() => pickRow(row)}
                    >
                      <span className="mm-check">{model.id === modelId && <Check size={13} aria-hidden />}</span>
                      <span className="mm-name">{model.name || model.model}</span>
                      <span className="mm-meta">{enabled ? '' : '已停用'}</span>
                    </button>
                  );
                })}
              </div>
            ))}
            {!groups.length && <div className="mm-empty">没有匹配的模型</div>}
          </div>
          {efforts.length > 0 && (
            <div className="mm-effort" role="group">
              <div className="me-label">
                <Zap size={14} aria-hidden />
                <span className="me-value">{efforts[activeIndex]}</span>
                <button
                  type="button"
                  className="me-reset"
                  aria-label="重置为最高档位"
                  title="重置为最高档位"
                  onClick={() => onPickEffort(efforts[efforts.length - 1])}
                >
                  <RotateCcw size={13} aria-hidden />
                </button>
              </div>
              <div className="me-slider-wrap">
                <div className="me-track" style={{ background: sliderFill }} />
                <div className="me-dots">
                  {efforts.map((_, index) => (
                    <span
                      key={index}
                      className={index <= activeIndex ? 'passed' : undefined}
                      style={{ left: `calc(8px + (100% - 16px) * ${efforts.length > 1 ? index / (efforts.length - 1) : 0})` }}
                    />
                  ))}
                </div>
                <input
                  type="range"
                  className="me-slider"
                  min={0}
                  max={efforts.length - 1}
                  step={1}
                  value={activeIndex}
                  aria-label="推理档位"
                  aria-valuetext={efforts[activeIndex]}
                  onChange={(e) => onPickEffort(efforts[Number(e.target.value)])}
                />
              </div>
            </div>
          )}
          <div className="mm-footer">
            <button type="button" className="mm-manage" onClick={onOpenManage}>
              <Settings2 size={13} aria-hidden />
              管理模型
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
