import { type ReactNode } from "react";
import { Routes, Route } from "react-router-dom";
import { motion } from "framer-motion";
import { Layout } from "./components/Layout";
import { SearchPage } from "./pages/SearchPage";
import { SourcesPage } from "./pages/SourcesPage";
import { PlaybooksPage } from "./pages/PlaybooksPage";
import { CommandPalette } from "./components/CommandPalette";

/* ── Page transition wrapper ─────────────────────────────────────── */
function PageTransition({ children }: { children: ReactNode }) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
    >
      {children}
    </motion.div>
  );
}

function App() {
  return (
    <>
      <CommandPalette />
      <Routes>
        <Route element={<Layout />}>
          <Route path="/" element={<PageTransition><SearchPage /></PageTransition>} />
          <Route path="/sources" element={<PageTransition><SourcesPage /></PageTransition>} />
          <Route path="/playbooks" element={<PageTransition><PlaybooksPage /></PageTransition>} />
        </Route>
      </Routes>
    </>
  );
}

export default App;
