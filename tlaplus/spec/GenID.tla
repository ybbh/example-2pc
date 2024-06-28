------------------------------ MODULE GenID ------------------------------

(*Self increasing ID*)

NextID ==
  (*************************************************************************)
  (* Generate a self increasing ID                                         *)
  (*************************************************************************)
  CHOOSE val : TRUE

GetID ==
  (*************************************************************************)
  (* Get current id                                                        *)
  (*************************************************************************)
  CHOOSE val : TRUE

SetID(v) ==
  (*************************************************************************)
  (* Set current id                                                        *)
  (*************************************************************************)
  TRUE
       
=============================================================================
