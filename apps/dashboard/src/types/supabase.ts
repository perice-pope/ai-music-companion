// Hand-authored against supabase/migrations/0006_teacher_dashboard_star_schema.sql
// (column-for-column) plus the 0001 `profiles` table, following the desktop
// convention (apps/desktop/src/types/supabase.ts, itself hand-extended for
// 0006). Scope is deliberately the dashboard's READ surface only: the tables
// and views the query→policy map in docs/specs/449-t4-dashboard.md §4 names.
// mv_student_day / mv_material_key are intentionally absent — they carry no
// client grant (0006 lines 667-670) and must never be queried from here.
// Regenerate/reconcile with `supabase gen types typescript` when the CLI is
// run against the applied migrations.

export type Json =
  | string
  | number
  | boolean
  | null
  | { [key: string]: Json | undefined }
  | Json[];

export type Database = {
  public: {
    Tables: {
      profiles: {
        Row: {
          age_tier: string | null;
          created_at: string;
          display_name: string | null;
          id: string;
          role: string;
        };
        Insert: {
          age_tier?: string | null;
          created_at?: string;
          display_name?: string | null;
          id: string;
          role?: string;
        };
        Update: {
          age_tier?: string | null;
          created_at?: string;
          display_name?: string | null;
          id?: string;
          role?: string;
        };
        Relationships: [];
      };
      classrooms: {
        Row: {
          created_at: string;
          id: string;
          join_code: string | null;
          join_code_expires_at: string | null;
          name: string;
          teacher_id: string;
        };
        Insert: {
          created_at?: string;
          id?: string;
          join_code?: string | null;
          join_code_expires_at?: string | null;
          name: string;
          teacher_id: string;
        };
        Update: {
          created_at?: string;
          id?: string;
          join_code?: string | null;
          join_code_expires_at?: string | null;
          name?: string;
          teacher_id?: string;
        };
        Relationships: [
          {
            foreignKeyName: "classrooms_teacher_id_fkey";
            columns: ["teacher_id"];
            isOneToOne: false;
            referencedRelation: "profiles";
            referencedColumns: ["id"];
          },
        ];
      };
      enrollments: {
        Row: {
          classroom_id: string;
          consent: string | null;
          consented_at: string | null;
          consenting_adult_id: string | null;
          created_at: string;
          id: string;
          joined_via_code_at: string | null;
          revoked_at: string | null;
          status: string;
          student_id: string;
        };
        Insert: {
          classroom_id: string;
          consent?: string | null;
          consented_at?: string | null;
          consenting_adult_id?: string | null;
          created_at?: string;
          id?: string;
          joined_via_code_at?: string | null;
          revoked_at?: string | null;
          status?: string;
          student_id: string;
        };
        Update: {
          classroom_id?: string;
          consent?: string | null;
          consented_at?: string | null;
          consenting_adult_id?: string | null;
          created_at?: string;
          id?: string;
          joined_via_code_at?: string | null;
          revoked_at?: string | null;
          status?: string;
          student_id?: string;
        };
        Relationships: [
          {
            foreignKeyName: "enrollments_classroom_id_fkey";
            columns: ["classroom_id"];
            isOneToOne: false;
            referencedRelation: "classrooms";
            referencedColumns: ["id"];
          },
          {
            foreignKeyName: "enrollments_student_id_fkey";
            columns: ["student_id"];
            isOneToOne: false;
            referencedRelation: "profiles";
            referencedColumns: ["id"];
          },
        ];
      };
      dim_material: {
        Row: {
          created_at: string;
          kind: string;
          label: string;
          material_id: string;
          score_id: string | null;
          source: string;
          spec_hash: string | null;
        };
        Insert: {
          created_at?: string;
          kind: string;
          label: string;
          material_id?: string;
          score_id?: string | null;
          source: string;
          spec_hash?: string | null;
        };
        Update: {
          created_at?: string;
          kind?: string;
          label?: string;
          material_id?: string;
          score_id?: string | null;
          source?: string;
          spec_hash?: string | null;
        };
        Relationships: [];
      };
      fact_session: {
        Row: {
          app_version: string | null;
          created_at: string;
          device_session_id: string;
          duration_secs: number;
          ended_at: string | null;
          fingerprint: Json | null;
          instrument: string | null;
          note_count: number | null;
          phrase_count: number;
          played_secs: number | null;
          practice_mode: string | null;
          score_material_id: string | null;
          session_id: string;
          silence_ratio: number | null;
          started_at: string;
          student_id: string;
        };
        Insert: {
          app_version?: string | null;
          created_at?: string;
          device_session_id: string;
          duration_secs: number;
          ended_at?: string | null;
          fingerprint?: Json | null;
          instrument?: string | null;
          note_count?: number | null;
          phrase_count?: number;
          played_secs?: number | null;
          practice_mode?: string | null;
          score_material_id?: string | null;
          session_id?: string;
          silence_ratio?: number | null;
          started_at: string;
          student_id: string;
        };
        Update: {
          app_version?: string | null;
          created_at?: string;
          device_session_id?: string;
          duration_secs?: number;
          ended_at?: string | null;
          fingerprint?: Json | null;
          instrument?: string | null;
          note_count?: number | null;
          phrase_count?: number;
          played_secs?: number | null;
          practice_mode?: string | null;
          score_material_id?: string | null;
          session_id?: string;
          silence_ratio?: number | null;
          started_at?: string;
          student_id?: string;
        };
        Relationships: [
          {
            foreignKeyName: "fact_session_student_id_fkey";
            columns: ["student_id"];
            isOneToOne: false;
            referencedRelation: "profiles";
            referencedColumns: ["id"];
          },
          {
            foreignKeyName: "fact_session_score_material_id_fkey";
            columns: ["score_material_id"];
            isOneToOne: false;
            referencedRelation: "dim_material";
            referencedColumns: ["material_id"];
          },
        ];
      };
      fact_phrase: {
        Row: {
          end_secs: number;
          key_name: string | null;
          note_count: number | null;
          phrase_index: number;
          session_id: string;
          stability: number | null;
          start_secs: number;
          tone: Json | null;
        };
        Insert: {
          end_secs: number;
          key_name?: string | null;
          note_count?: number | null;
          phrase_index: number;
          session_id: string;
          stability?: number | null;
          start_secs: number;
          tone?: Json | null;
        };
        Update: {
          end_secs?: number;
          key_name?: string | null;
          note_count?: number | null;
          phrase_index?: number;
          session_id?: string;
          stability?: number | null;
          start_secs?: number;
          tone?: Json | null;
        };
        Relationships: [
          {
            foreignKeyName: "fact_phrase_session_id_fkey";
            columns: ["session_id"];
            isOneToOne: false;
            referencedRelation: "fact_session";
            referencedColumns: ["session_id"];
          },
        ];
      };
      fact_exercise: {
        Row: {
          accuracy: number | null;
          created_at: string;
          device_log_id: number | null;
          difficulty: number;
          exercise_id: string;
          logged_at: string;
          material_id: string;
          session_id: string | null;
          student_id: string;
          tonic: number;
        };
        Insert: {
          accuracy?: number | null;
          created_at?: string;
          device_log_id?: number | null;
          difficulty: number;
          exercise_id?: string;
          logged_at: string;
          material_id: string;
          session_id?: string | null;
          student_id: string;
          tonic: number;
        };
        Update: {
          accuracy?: number | null;
          created_at?: string;
          device_log_id?: number | null;
          difficulty?: number;
          exercise_id?: string;
          logged_at?: string;
          material_id?: string;
          session_id?: string | null;
          student_id?: string;
          tonic?: number;
        };
        Relationships: [
          {
            foreignKeyName: "fact_exercise_student_id_fkey";
            columns: ["student_id"];
            isOneToOne: false;
            referencedRelation: "profiles";
            referencedColumns: ["id"];
          },
          {
            foreignKeyName: "fact_exercise_material_id_fkey";
            columns: ["material_id"];
            isOneToOne: false;
            referencedRelation: "dim_material";
            referencedColumns: ["material_id"];
          },
          {
            foreignKeyName: "fact_exercise_session_id_fkey";
            columns: ["session_id"];
            isOneToOne: false;
            referencedRelation: "fact_session";
            referencedColumns: ["session_id"];
          },
        ];
      };
      fact_tool_event: {
        Row: {
          at_secs: number;
          created_at: string;
          device_event_id: number | null;
          event_id: string;
          kind: string;
          params: Json;
          session_id: string;
          student_id: string;
        };
        Insert: {
          at_secs: number;
          created_at?: string;
          device_event_id?: number | null;
          event_id?: string;
          kind: string;
          params?: Json;
          session_id: string;
          student_id: string;
        };
        Update: {
          at_secs?: number;
          created_at?: string;
          device_event_id?: number | null;
          event_id?: string;
          kind?: string;
          params?: Json;
          session_id?: string;
          student_id?: string;
        };
        Relationships: [
          {
            foreignKeyName: "fact_tool_event_session_id_fkey";
            columns: ["session_id"];
            isOneToOne: false;
            referencedRelation: "fact_session";
            referencedColumns: ["session_id"];
          },
          {
            foreignKeyName: "fact_tool_event_student_id_fkey";
            columns: ["student_id"];
            isOneToOne: false;
            referencedRelation: "profiles";
            referencedColumns: ["id"];
          },
        ];
      };
    };
    Views: {
      // 0006 lines 703-719: security_invoker roster-heat view. One row per
      // (classroom, student, day) with the played-vs-wall pair — "the gap IS
      // the story".
      v_roster_heat: {
        Row: {
          classroom_id: string;
          date_key: string;
          display_name: string | null;
          integrity_flag: boolean | null;
          played_min: number | null;
          sessions: number;
          student_id: string;
          thin_sessions: number;
          wall_min: number | null;
        };
        Relationships: [];
      };
      // 0006 lines 725-744: security_invoker integrity view — evidence rows,
      // never verdicts (no column says "cheating", by design).
      v_session_integrity: {
        Row: {
          display_name: string | null;
          graded: number | null;
          max_retries: number | null;
          note_count: number | null;
          phrase_count: number;
          played_min: number | null;
          session_id: string;
          silence_ratio: number | null;
          started_at: string;
          student_id: string;
          wall_min: number | null;
        };
        Relationships: [];
      };
    };
    Functions: {
      [_ in never]: never;
    };
    Enums: {
      [_ in never]: never;
    };
    CompositeTypes: {
      [_ in never]: never;
    };
  };
};
