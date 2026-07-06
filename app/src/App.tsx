import { useCallback, useEffect, useState } from 'react';
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  type Connection,
  type Node,
  type Edge,
  Position,
  Handle,
  type NodeProps,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { invoke } from '@tauri-apps/api/core';

// --- Types matching Rust backend ---

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
  edges: { source_id: string; target_id: string }[];
};

// --- Custom node component ---

const transportColors: Record<TransportKind, string> = {
  local: '#3b82f6',
  virtual: '#a855f7',
  network: '#f59e0b',
};

function AudioNode({ data }: NodeProps) {
  const nodeData = data as { label: string; transport: TransportKind; direction: NodeDirection };
  const color = transportColors[nodeData.transport];
  const isOutput = nodeData.direction === 'output';
  const isInput = nodeData.direction === 'input';

  return (
    <div
      style={{
        padding: '10px 16px',
        borderRadius: 8,
        background: '#1e2330',
        border: `2px solid ${color}`,
        minWidth: 180,
        fontSize: 13,
      }}
    >
      {isInput && (
        <Handle
          type="target"
          position={Position.Left}
          style={{ background: color, width: 10, height: 10 }}
        />
      )}
      <div style={{ fontWeight: 600, color: '#e2e8f0' }}>{nodeData.label}</div>
      <div style={{ fontSize: 11, color: '#94a3b8', marginTop: 2 }}>
        {nodeData.transport} · {nodeData.direction}
      </div>
      {isOutput && (
        <Handle
          type="source"
          position={Position.Right}
          style={{ background: color, width: 10, height: 10 }}
        />
      )}
    </div>
  );
}

const nodeTypes = { audioNode: AudioNode };

// --- Helpers ---

function buildFlowNodes(routeNodes: RouteNode[]): Node[] {
  const outputs = routeNodes.filter((n) => n.direction === 'output');
  const inputs = routeNodes.filter((n) => n.direction === 'input');

  const ySpacing = 80;
  const leftX = 50;
  const rightX = 450;

  const flowNodes: Node[] = [];

  outputs.forEach((node, i) => {
    flowNodes.push({
      id: node.id,
      type: 'audioNode',
      position: { x: leftX, y: 40 + i * ySpacing },
      data: { label: node.name, transport: node.transport, direction: node.direction },
    });
  });

  inputs.forEach((node, i) => {
    flowNodes.push({
      id: node.id,
      type: 'audioNode',
      position: { x: rightX, y: 40 + i * ySpacing },
      data: { label: node.name, transport: node.transport, direction: node.direction },
    });
  });

  return flowNodes;
}

function buildFlowEdges(routeEdges: { source_id: string; target_id: string }[]): Edge[] {
  return routeEdges.map((e, i) => ({
    id: `edge-${i}`,
    source: e.source_id,
    target: e.target_id,
    animated: true,
    style: { stroke: '#64748b', strokeWidth: 2 },
  }));
}

function isNetworkNode(id: string, routeNodes: RouteNode[]): boolean {
  const node = routeNodes.find((n) => n.id === id);
  return node?.transport === 'network';
}

// --- Main App ---

function App() {
  const [routeNodes, setRouteNodes] = useState<RouteNode[]>([]);
  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [statusMessage, setStatusMessage] = useState('');

  useEffect(() => {
    invoke<RouteGraph>('get_route_graph').then((graph) => {
      setRouteNodes(graph.nodes);
      setNodes(buildFlowNodes(graph.nodes));
      setEdges(buildFlowEdges(graph.edges));
    });
  }, [setNodes, setEdges]);

  const onConnect = useCallback(
    async (connection: Connection) => {
      const sourceId = connection.source;
      const targetId = connection.target;
      if (!sourceId || !targetId) return;

      if (isNetworkNode(sourceId, routeNodes) && isNetworkNode(targetId, routeNodes)) {
        setStatusMessage('Cannot connect network nodes to each other.');
        return;
      }

      try {
        const graph = await invoke<RouteGraph>('add_route', {
          edge: { sourceId, targetId },
        });
        setEdges(buildFlowEdges(graph.edges));
        setStatusMessage('');
      } catch (err) {
        setStatusMessage(`Route rejected: ${String(err)}`);
      }
    },
    [routeNodes, setEdges],
  );

  const onEdgeDoubleClick = useCallback(
    async (_event: React.MouseEvent, edge: Edge) => {
      const graph = await invoke<RouteGraph>('remove_route', {
        sourceId: edge.source,
        targetId: edge.target,
      });
      setEdges(buildFlowEdges(graph.edges));
    },
    [setEdges],
  );

  const refreshDevices = useCallback(async () => {
    const graph = await invoke<RouteGraph>('refresh_audio_devices');
    setRouteNodes(graph.nodes);
    setNodes(buildFlowNodes(graph.nodes));
    setEdges(buildFlowEdges(graph.edges));
    setStatusMessage('Devices refreshed.');
  }, [setNodes, setEdges]);

  return (
    <div style={{ width: '100vw', height: '100vh', display: 'flex', flexDirection: 'column' }}>
      <header
        style={{
          padding: '12px 20px',
          background: '#0f1219',
          borderBottom: '1px solid #1e293b',
          display: 'flex',
          alignItems: 'center',
          gap: 16,
        }}
      >
        <h1 style={{ margin: 0, fontSize: 18, color: '#e2e8f0' }}>GugleAudio</h1>
        <span style={{ color: '#64748b', fontSize: 13 }}>
          Outputs (left) → Inputs (right) · Drag to connect · Double-click edge to remove
        </span>
        <button
          onClick={refreshDevices}
          style={{
            marginLeft: 'auto',
            padding: '6px 14px',
            borderRadius: 6,
            border: '1px solid #334155',
            background: '#1e293b',
            color: '#e2e8f0',
            cursor: 'pointer',
            fontSize: 13,
          }}
        >
          Refresh Devices
        </button>
      </header>

      {statusMessage && (
        <div
          style={{
            padding: '8px 20px',
            background: '#7c2d12',
            color: '#fef2f2',
            fontSize: 13,
          }}
        >
          {statusMessage}
        </div>
      )}

      <div style={{ flex: 1 }}>
        <ReactFlow
          nodes={nodes}
          edges={edges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          onEdgeDoubleClick={onEdgeDoubleClick}
          nodeTypes={nodeTypes}
          fitView
          style={{ background: '#0f1219' }}
          defaultEdgeOptions={{ animated: true, style: { stroke: '#64748b', strokeWidth: 2 } }}
        >
          <Background color="#1e293b" gap={20} />
          <Controls />
          <MiniMap
            nodeColor={(node) => {
              const transport = (node.data as { transport: TransportKind }).transport;
              return transportColors[transport] ?? '#64748b';
            }}
            style={{ background: '#1a1f2e' }}
          />
        </ReactFlow>
      </div>
    </div>
  );
}

export default App;
