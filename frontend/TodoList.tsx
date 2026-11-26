const printable = (value: unknown) =>
  typeof value === "string" ? value : JSON.stringify(value, null, 2);

export function TodoList({ items }: { items: Record<string, unknown>[] }) {
  return items.length ? (
    <ul className="todo-list">
      {items.map((item, index) => {
        const text = String(item.text ?? item.content ?? item.title ?? printable(item));
        const done = item.done === true || item.completed === true || item.status === "completed";
        return (
          <li key={index} className={done ? "done" : undefined}>
            {done ? "✓" : "○"} {text}
          </li>
        );
      })}
    </ul>
  ) : (
    <p className="muted">清单为空。</p>
  );
}
