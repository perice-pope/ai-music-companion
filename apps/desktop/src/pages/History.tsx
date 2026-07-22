import { useEffect } from "react";
import { useHistoryStore } from "../stores/historyStore";
import { usePracticeStore } from "../stores/practiceStore";
import SessionCard from "../components/SessionCard";
import PastSessionDetail from "../components/PastSessionDetail";
import PracticeStats from "../components/PracticeStats";
import AccountPanel from "../components/AccountPanel";
import ClassroomPanel from "../components/ClassroomPanel";

export default function History() {
  const {
    sessions,
    stats,
    isLoading,
    error,
    loadHistory,
    loadStats,
    loadSessionDetail,
    selectedSessionDetail,
    clearSelectedSession,
  } = useHistoryStore();
  const goToConnections = usePracticeStore((s) => s.goToConnections);
  const goToSelector = usePracticeStore((s) => s.goToSelector);

  useEffect(() => {
    // Review SF3: re-entering History lands on the LIST — a detail left
    // open last visit must not greet you as stale navigation state.
    clearSelectedSession();
    loadHistory();
    loadStats();
  }, [clearSelectedSession, loadHistory, loadStats]);

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
        {/* #445-8: History must not be a dead end — back to practice. */}
        <button
          type="button"
          onClick={() => goToSelector()}
          data-testid="history-back"
          className="mb-4 text-sm text-gray-400 underline-offset-2 hover:text-gray-200 hover:underline"
        >
          ← Back to practice
        </button>
        <h1 className="mb-8 text-4xl font-bold text-white">My sessions</h1>

        {/* Cloud sync (optional sign-in) */}
        <section className="mb-12">
          <AccountPanel />
          {/* #449 enrollment: classroom join/leave + the teacher card.
              Renders nothing signed out. */}
          <ClassroomPanel />
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

        {/* Sessions List — or the open past recap (#445-8: tapping a card
            used to fetch the detail into state and render NOTHING). */}
        <section>
          <h2 className="mb-4 text-2xl font-semibold text-gray-200">
            Sessions
          </h2>

          {/* Review MF2: a detail-fetch failure must be VISIBLE next to
              the list — a store-only error is the dead-tap pattern this
              page exists to kill. */}
          {error && sessions.length > 0 && !selectedSessionDetail && (
            <p
              className="mb-3 rounded-md border border-amber-700 bg-amber-900/30 px-3 py-2 text-sm text-amber-200"
              data-testid="history-error"
            >
              {error}
            </p>
          )}
          {selectedSessionDetail ? (
            <PastSessionDetail
              session={selectedSessionDetail}
              onBack={clearSelectedSession}
            />
          ) : sessions.length === 0 ? (
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
