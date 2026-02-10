import { NavLink, Outlet } from "react-router-dom";

const navItems = [
  { to: "/", label: "Search", icon: "🔍" },
  { to: "/sources", label: "Sources", icon: "📁" },
  { to: "/playbooks", label: "Playbooks", icon: "📋" },
];

export function Layout() {
  return (
    <div className="flex h-screen bg-gray-950 text-gray-100">
      {/* Sidebar */}
      <aside className="flex w-52 shrink-0 flex-col border-r border-gray-800 bg-gray-900">
        <div className="px-4 py-5">
          <h1 className="text-lg font-bold tracking-tight">Ask Myself</h1>
          <p className="text-xs text-gray-500">Evidence recall engine</p>
        </div>

        <nav className="flex-1 space-y-1 px-2">
          {navItems.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.to === "/"}
              className={({ isActive }) =>
                `flex items-center gap-2 rounded-md px-3 py-2 text-sm transition ${
                  isActive
                    ? "bg-gray-800 text-white"
                    : "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200"
                }`
              }
            >
              <span>{item.icon}</span>
              <span>{item.label}</span>
            </NavLink>
          ))}
        </nav>

        <div className="border-t border-gray-800 px-4 py-3 text-xs text-gray-600">
          v0.1.0
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-y-auto">
        <Outlet />
      </main>
    </div>
  );
}
