import { TypedUseSelectorHook, useDispatch, useSelector } from "react-redux";
import { AppDispatch, RootState } from ".";
import { ICastMemberIdentifier } from "../vm";
import { useEffect, useRef } from "react";
import { TMemberSubscription, memberSubscribed, memberUnsubscribed, selectMemberSnapshotById } from "./vmSlice";
import { uniqueId } from "lodash";
import { subscribe_to_member, unsubscribe_from_member } from "vm-rust";
import { ICastMemberRef } from "dirplayer-js-api";

export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector
export const useAppDispatch: () => AppDispatch = useDispatch

export const useMemberSnapshot = (memberRef: ICastMemberIdentifier) => {
  const dispatch = useAppDispatch();
  useEffect(() => {
    const token: TMemberSubscription = {
      memberRef: {
        castNumber: memberRef.castNumber,
        memberNumber: memberRef.memberNumber,
      },
      id: uniqueId("memberSubscription_"),
    };
    dispatch(memberSubscribed(token))
    return () => {
      dispatch(memberUnsubscribed(token.id))
    }
  }, [memberRef.castNumber, memberRef.memberNumber, dispatch]);
  return useAppSelector(state => selectMemberSnapshotById(state.vm, memberRef))
}

export const useMemberSubscriptions = () => {
  const subscriptions = useAppSelector(state => state.vm.subscribedMemberTokens)
  const subscribedIds = useRef<ICastMemberRef[]>([])
  useEffect(() => {
    const newSubs = subscriptions.filter(sub => !subscribedIds.current.some(id => id[0] === sub.memberRef.castNumber && id[1] === sub.memberRef.memberNumber))
    const oldSubs = subscribedIds.current.filter(id => !subscriptions.some(sub => sub.memberRef.castNumber === id[0] && sub.memberRef.memberNumber === id[1]))
    newSubs.forEach(sub => {
      subscribe_to_member(sub.memberRef.castNumber, sub.memberRef.memberNumber)
      subscribedIds.current.push([sub.memberRef.castNumber, sub.memberRef.memberNumber])
    })
    oldSubs.forEach(sub => {
      unsubscribe_from_member(sub[0], sub[1])
      subscribedIds.current = subscribedIds.current.filter(id => id !== sub)
    })
  }, [subscriptions])
}