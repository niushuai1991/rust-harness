import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { join } from "node:path";

const root = process.env.MCP_HTTP_INTEROP_MODULES;
if (!root) throw new Error("MCP_HTTP_INTEROP_MODULES is required");
const resolve = createRequire(join(root, "package.json"));
const load = (name) => import(pathToFileURL(resolve.resolve(name)).href);
const { default: express } = await load("express");
const { Server } = await load("@modelcontextprotocol/sdk/server/index.js");
const { StreamableHTTPServerTransport } = await load("@modelcontextprotocol/sdk/server/streamableHttp.js");
const { ListToolsRequestSchema, CallToolRequestSchema } = await load("@modelcontextprotocol/sdk/types.js");

const app = express();
app.use(express.json());
app.post("/mcp", async (req, res) => {
  const server = new Server({ name: "http-interop", version: "1" }, { capabilities: { tools: {} } });
  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: [{ name: "echo", description: "Echo input", inputSchema: { type: "object", properties: { text: { type: "string" } }, required: ["text"] } }],
  }));
  server.setRequestHandler(CallToolRequestSchema, async (request) => ({
    content: [{ type: "text", text: request.params.arguments.text }],
  }));
  const transport = new StreamableHTTPServerTransport({ sessionIdGenerator: undefined });
  res.on("close", () => { void transport.close(); void server.close(); });
  await server.connect(transport);
  await transport.handleRequest(req, res, req.body);
});
app.get("/mcp", (_req, res) => res.status(405).end());
app.delete("/mcp", (_req, res) => res.status(405).end());
const listener = app.listen(0, "127.0.0.1", () => process.stdout.write(String(listener.address().port) + "\n"));
