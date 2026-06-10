import { useEffect } from "react";
import { useHistoryStore } from "../stores/historyStore";
import { usePracticeStore } from "../stores/practiceStore";
import SessionCard from "../components/SessionCard";
import PracticeStats from "../components/PracticeStats";
import AccountPanel from "../components/AccountPanel";

export default function History() {
  const {
    sessions,
    stats,
    isLoading,
    error,
    loadHistory,
    loadStats,
    loadSessionDetail,
  } = useHistoryStore();
  const goToConnections = usePracticeStore((s) => s.goToConnections);

  useEffect(() => {
    loadHistory();
    loadStats();
  }, [loadHistory, loadStats]);

  if (isLoading && !stats) {
    return (
      <main className="min-h-screen bg-gray-900 p-8 text-white">
        <div className="text-center">
          <p className="text-gray-400">Loading...</p>
        </div>
      </main>
    );
  }

  if (error && !sessions.length) {
    return (
      <main className="min-h-screen bg-gray-900 p-8 text-white">
        <div className="rounded-lg bg-red-900/20 border border-red-500 p-4">
          <p className="text-red-300">Error: {error}</p>
        </div>
      </main>
    );
  }

  return (
    <main className="min-h-screen bg-gray-900 p-8">
      <div className="mx-auto max-w-4xl">
        <h1 className="mb-8 text-4xl font-bold text-white">Practice History</h1>

        {/* Cloud sync (optional sign-in) */}
        <section className="mb-12">
          <AccountPanel />
          <button
            onClick={() => goToConnections()}
            className="mt-3 text-sm text-gray-400 underline-offset-2 hover:text-gray-200 hover:underline"
          >
            Connections &amp; Privacy — what uses the internet
          </button>
        </section>

        {/* Dashboard */}
        {stats && (
          <section className="mb-12">
            <h2 className="mb-4 text-2xl font-semibold text-gray-200">
              Overview
            </h2>
            <PracticeStats stats={stats} />
          </section>
        )}

        {/* Sessions List */}
        <section>
          <h2 className="mb-4 text-2xl font-semibold text-gray-200">
            Sessions
          </h2>

          {sessions.length === 0 ? (
            <div className="rounded-lg bg-gray-800 p-8 text-center">
              <p className="text-gray-400">No sessions yet.</p>
              <p className="mt-2 text-sm text-gray-500">
                Start a practice session to see your history here.
              </p>
            </div>
          ) : (
            <div className="space-y-3">
              {sessions.map((session) => (
                <SessionCard
                  key={session.id}
                  session={session}
                  onClick={() => loadSessionDetail(session.id)}
                />
              ))}
            </div>
          )}
        </section>
      </div>
    </main>
  );
}
