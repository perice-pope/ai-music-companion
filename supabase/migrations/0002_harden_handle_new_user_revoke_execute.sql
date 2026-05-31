-- handle_new_user() is only meant to fire from the on_auth_user_created trigger,
-- never to be called directly via the REST RPC endpoint. The trigger still fires
-- regardless of these grants; revoking EXECUTE just closes the public RPC surface
-- (clears the Supabase security advisor warnings 0028/0029).
revoke execute on function public.handle_new_user() from anon, authenticated, public;
