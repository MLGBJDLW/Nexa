import { Routes, Route } from "react-router-dom";
import { Layout } from "./components/Layout";
import { SearchPage } from "./pages/SearchPage";
import { SourcesPage } from "./pages/SourcesPage";
import { PlaybooksPage } from "./pages/PlaybooksPage";

function App() {
  return (
    <Routes>
      <Route element={<Layout />}>
        <Route path="/" element={<SearchPage />} />
        <Route path="/sources" element={<SourcesPage />} />
        <Route path="/playbooks" element={<PlaybooksPage />} />
      </Route>
    </Routes>
  );
}

export default App;
