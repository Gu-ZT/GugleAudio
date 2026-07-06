import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './styles.css';

type TransportKind = 'local' | 'virtual' | 'network';
type NodeDirection = 'input' | 'output';

type RouteNode = {
  id: string;
  name: string;
  transport: TransportKind;
  direction: NodeDirection;
};

type RouteGraph = {
  nodes: RouteNode[];
  edges: { sourceId: string; targetId: string }[];
};

type AudioDeviceInfo = {
  id: string;
  name: string;
  flow: string;
  role: string;
};

const COLORS = [
  '#a78bfa', '#f97316', '#06b6d4', '#f43f5e',
  '#22c55e', '#eab308', '#3b82f6', '#ec4899',
];

function App() {
  const [graph, setGraph] = useState<RouteGraph | null>(null);
  const [allDevices, setAllDevices] = useState<AudioDeviceInfo[]>([]);
  const [activeInputs, setActiveInputs] = useState<string[]>([]);
  const [activeOutputs, setActiveOutputs] = useState<string[]>([]);
  const [volumes, setVolumes] = useState<Record<string, number>>({});
  const [statusMessage, setStatusMessage] = useState('');
  const [, setTick] = useState(0);
  const [dragging, setDragging] = useState<{
    sourceId: string; startX: number; startY: number; curX: number; curY: number;
  } | null>(null);
  const panelsRef = useRef<HTMLDivElement>(null);

  const loadData = useCallback(async () => {
    const [g, devices] = await Promise.all([
      invoke<RouteGraph>('get_route_graph'),
      invoke<AudioDeviceInfo[]>('get_audio_devices'),
    ]);
    setGraph(g);
    setAllDevices(devices);
  }, []);

  useEffect(() => { loadData(); }, [loadData]);
  useLayoutEffect(() => {
    const id = requestAnimationFrame(() => setTick((t) => t + 1));
    return () => cancelAnimationFrame(id);
  }, [graph, activeInputs, activeOutputs]);

  // Nodes
  const inputNodes = graph?.nodes.filter((n) => n.direction === 'output') ?? [];
  const outputNodes = graph?.nodes.filter((n) => n.direction === 'input') ?? [];
  const activeInNodes = inputNodes.filter((n) => activeInputs.includes(n.id));
  const activeOutNodes = outputNodes.filter((n) => activeOutputs.includes(n.id));
  const availInputs = inputNodes.filter((n) => !activeInputs.includes(n.id));
  const availOutputs = outputNodes.filter((n) => !activeOutputs.includes(n.id));

  // Volume
  const vol = (key: string) => volumes[key] ?? 100;
  const setVol = (key: string, v: number) => setVolumes((p) => ({ ...p, [key]: v }));

  // Dot positions
  const dotPos = (dotId: string) => {
    const el = document.getElementById(dotId);
    const c = panelsRef.current;
    if (!el || !c) return null;
    const er = el.getBoundingClientRect();
    const cr = c.getBoundingClientRect();
    return { x: er.left + er.width / 2 - cr.left, y: er.top + er.height / 2 - cr.top + c.scrollTop };
  };

  // Drag from + button to create new connection
  const startDrag = (e: React.MouseEvent, sourceId: string) => {
    e.preventDefault();
    const c = panelsRef.current;
    if (!c) return;
    const cr = c.getBoundingClientRect();
    const el = e.currentTarget as HTMLElement;
    const er = el.getBoundingClientRect();
    const sx = er.left + er.width / 2 - cr.left;
    const sy = er.top + er.height / 2 - cr.top + c.scrollTop;
    setDragging({ sourceId, startX: sx, startY: sy, curX: e.clientX - cr.left, curY: e.clientY - cr.top + c.scrollTop });
  };

  const onMove = (e: React.MouseEvent) => {
    if (!dragging || !panelsRef.current) return;
    const cr = panelsRef.current.getBoundingClientRect();
    setDragging({ ...dragging, curX: e.clientX - cr.left, curY: e.clientY - cr.top + panelsRef.current.scrollTop });
  };

  const onUp = async (e: React.MouseEvent) => {
    if (!dragging) return;
    const el = (e.target as HTMLElement).closest('[data-input-id]');
    const tid = el?.getAttribute('data-input-id');
    if (tid) {
      const srcNet = graph?.nodes.find((n) => n.id === dragging.sourceId)?.transport === 'network';
      const tgtNet = graph?.nodes.find((n) => n.id === tid)?.transport === 'network';
      if (srcNet && tgtNet) {
        setStatusMessage('Network → Network forbidden');
      } else {
        try {
          const g = await invoke<RouteGraph>('add_route', { edge: { sourceId: dragging.sourceId, targetId: tid } });
          setGraph(g);
          setStatusMessage('');
        } catch (err) { setStatusMessage(String(err)); }
      }
    }
    setDragging(null);
  };

  const delEdge = async (s: string, t: string) => {
    const g = await invoke<RouteGraph>('remove_route', { sourceId: s, targetId: t });
    setGraph(g);
  };

  const addInput = (id: string) => setActiveInputs((p) => [...p, id]);
  const addOutput = (id: string) => setActiveOutputs((p) => [...p, id]);
  const removeInput = async (id: string) => {
    setActiveInputs((p) => p.filter((x) => x !== id));
    const edges = graph?.edges.filter((e) => e.sourceId === id) ?? [];
    for (const e of edges) await invoke('remove_route', { sourceId: e.sourceId, targetId: e.targetId });
    loadData();
  };
  const removeOutput = async (id: string) => {
    setActiveOutputs((p) => p.filter((x) => x !== id));
    const edges = graph?.edges.filter((e) => e.targetId === id) ?? [];
    for (const e of edges) await invoke('remove_route', { sourceId: e.sourceId, targetId: e.targetId });
    loadData();
  };

  // Get color for an edge based on global edge index
  const getEdgeColor = (sourceId: string, targetId: string) => {
    const allEdges = graph?.edges ?? [];
    const idx = allEdges.findIndex((e) => e.sourceId === sourceId && e.targetId === targetId);
    return COLORS[(idx >= 0 ? idx : 0) % COLORS.length];
  };

  // SVG
  const renderSvg = () => {
    if (!graph) return null;
    const els: React.ReactNode[] = [];
    const visible = graph.edges.filter(
      (e) => activeInputs.includes(e.sourceId) && activeOutputs.includes(e.targetId),
    );

    visible.forEach((edge, i) => {
      const dotOutId = `dot-conn-${edge.sourceId}-${edge.targetId}`;
      const dotInId = `dot-in-${edge.targetId}`;
      const s = dotPos(dotOutId);
      const t = dotPos(dotInId);
      if (!s || !t) return;
      const col = COLORS[i % COLORS.length];
      const cp = Math.max(80, Math.abs(t.x - s.x) * 0.45);
      const d = `M${s.x},${s.y} C${s.x + cp},${s.y} ${t.x - cp},${t.y} ${t.x},${t.y}`;
      els.push(
        <g key={i} onClick={() => delEdge(edge.sourceId, edge.targetId)} style={{ cursor: 'pointer' }}>
          <path d={d} stroke={col} strokeWidth={3} fill="none" opacity={0.85} />
          <circle cx={s.x} cy={s.y} r={6} fill={col} />
          <circle cx={t.x} cy={t.y} r={6} fill={col} />
        </g>,
      );
    });

    if (dragging) {
      const cp = Math.max(60, Math.abs(dragging.curX - dragging.startX) * 0.4);
      const d = `M${dragging.startX},${dragging.startY} C${dragging.startX + cp},${dragging.startY} ${dragging.curX - cp},${dragging.curY} ${dragging.curX},${dragging.curY}`;
      els.push(<path key="preview" d={d} stroke="#64748b" strokeWidth={2} fill="none" strokeDasharray="5 4" />);
    }
    return els;
  };

  return (
    <div className="app-root" onMouseMove={onMove} onMouseUp={onUp}>
      <header className="hdr">
        <h1 className="hdr-title">GugleAudio</h1>
      </header>
      {statusMessage && <div className="toast" onClick={() => setStatusMessage('')}>{statusMessage}</div>}

      <div className="panels" ref={panelsRef}>
        {/* LEFT: Inputs */}
        <div className="panel left">
          <select className="add-select" value="" onChange={(e) => { if (e.target.value) addInput(e.target.value); }}>
            <option value="" disabled>Add Input</option>
            {availInputs.map((n) => <option key={n.id} value={n.id}>{n.name}</option>)}
          </select>

          {activeInNodes.map((node) => {
            const conns = graph?.edges.filter((e) => e.sourceId === node.id && activeOutputs.includes(e.targetId)) ?? [];
            return (
              <div key={node.id} className="card input-card">
                <div className="card-top">
                  <span className="card-name">{node.name}</span>
                  <button className="card-rm" onClick={() => removeInput(node.id)}>×</button>
                </div>

                {/* Per-connection rows with individual dots */}
                {conns.map((c) => {
                  const tgt = graph?.nodes.find((n) => n.id === c.targetId);
                  const k = `${c.sourceId}>${c.targetId}`;
                  const color = getEdgeColor(c.sourceId, c.targetId);
                  return (
                    <div key={k} className="conn-row">
                      <div className="conn-info">
                        <span className="conn-name">{tgt?.name}</span>
                        <div className="vol-wrap">
                          <div className="vol-meter" style={{ width: `${Math.random() * 50 + 10}%` }} />
                          <input type="range" min={0} max={100} value={vol(k)} onChange={(e) => setVol(k, +e.target.value)} className="vol-slider" />
                        </div>
                      </div>
                      <div
                        className="conn-dot"
                        id={`dot-conn-${c.sourceId}-${c.targetId}`}
                        style={{ borderColor: color }}
                      />
                    </div>
                  );
                })}

                {/* Add connection button (⊕) */}
                <div className="add-conn-row">
                  <div
                    className="add-conn-btn"
                    onMouseDown={(e) => startDrag(e, node.id)}
                  >⊕</div>
                </div>
              </div>
            );
          })}
        </div>

        <svg className={`svg-layer ${dragging ? 'dragging' : ''}`}>{renderSvg()}</svg>

        {/* RIGHT: Outputs */}
        <div className="panel right">
          <select className="add-select" value="" onChange={(e) => { if (e.target.value) addOutput(e.target.value); }}>
            <option value="" disabled>Add Output</option>
            {availOutputs.map((n) => <option key={n.id} value={n.id}>{n.name}</option>)}
          </select>

          {activeOutNodes.map((node) => {
            const k = `out-${node.id}`;
            return (
              <div key={node.id} className="card output-card" data-input-id={node.id}>
                <div className="out-dot-area" data-input-id={node.id}>
                  <div className="conn-dot out-dot" id={`dot-in-${node.id}`} data-input-id={node.id} />
                </div>
                <div className="out-content">
                  <div className="card-top">
                    <span className="card-name">{node.name}</span>
                    <button className="card-rm" onClick={() => removeOutput(node.id)}>×</button>
                  </div>
                  <div className="vol-wrap">
                    <div className="vol-meter" style={{ width: `${Math.random() * 50 + 10}%` }} />
                    <input type="range" min={0} max={100} value={vol(k)} onChange={(e) => setVol(k, +e.target.value)} className="vol-slider" />
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

export default App;
