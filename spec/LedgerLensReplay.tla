---------------------------- MODULE LedgerLensReplay ----------------------------
EXTENDS LedgerLens

VARIABLE replay_paused

ReplayVars == <<vars, replay_paused>>

ReplayInit ==
    /\ Init
    /\ replay_paused = FALSE

PauseContract ==
    /\ replay_paused = FALSE
    /\ replay_paused' = TRUE
    /\ UNCHANGED vars

UnpauseContract ==
    /\ replay_paused = TRUE
    /\ replay_paused' = FALSE
    /\ UNCHANGED vars

ReplayTickTime ==
    /\ (gov_proposal_id = 0 \/ gov_vetoed \/ gov_executed \/ now < gov_expiry)
    /\ TickTime
    /\ UNCHANGED replay_paused

ReplaySubmitScore(w, s) ==
    /\ ~replay_paused
    /\ SubmitScore(w, s)
    /\ UNCHANGED replay_paused
ReplayProposeGov(id, action) ==
    /\ gov_proposal_id = 0
    /\ action = 1
    /\ ProposeGov(id, action)
    /\ UNCHANGED replay_paused
ReplayExecuteGov == ExecuteGov /\ UNCHANGED replay_paused
ReplayMutateAdminSet(s) == MutateAdminSet(s) /\ UNCHANGED replay_paused

ReplayNext ==
    \/ ReplayTickTime
    \/ (\E w \in Wallets, s \in Scores : ReplaySubmitScore(w, s))
    \/ (\E id \in 1..4, action \in Actions : ReplayProposeGov(id, action))
    \/ ReplayExecuteGov
    \/ PauseContract
    \/ UnpauseContract
    \/ (\E s \in Signers : ReplayMutateAdminSet(s))

ReplaySpec == ReplayInit /\ [][ReplayNext]_ReplayVars

=============================================================================