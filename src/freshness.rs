use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
/// Review schedules describe evidence, independently of live ledger comparisons.
pub fn summary(evidence: Option<&Value>, deployment: Option<&Value>) -> Value {
    let now = Utc::now();
    let sources: Vec<Value> = evidence
        .into_iter()
        .flat_map(|e| {
            ["sources", "underlyingSources"]
                .into_iter()
                .flat_map(move |key| e[key].as_array().into_iter().flatten())
        })
        .map(|source| {
            let review = source["reviewAfter"].as_str().and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .map(|d| d.with_timezone(&Utc))
                    .ok()
                    .or_else(|| {
                        NaiveDate::parse_from_str(s, "%Y-%m-%d")
                            .ok()
                            .and_then(|d| d.and_hms_opt(0, 0, 0))
                            .map(|d| d.and_utc())
                    })
            });
            let status = match review {
                Some(date) if date.with_timezone(&Utc) <= now => "overdue",
                Some(_) => "scheduled",
                None => "unknown",
            };
            json!({"id": source["id"],"reviewAfter":source["reviewAfter"],"reviewStatus":status})
        })
        .collect();
    let recorded = deployment
        .and_then(|d| d["verification"]["lastVerifiedAt"].as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok());
    json!({"assessedAt":now.to_rfc3339(),"overdueSources":sources.iter().filter(|s|s["reviewStatus"]=="overdue").count(),"unknownReviewSchedules":sources.iter().filter(|s|s["reviewStatus"]=="unknown").count(),"sources":sources,"recordedObservationAgeSeconds":recorded.and_then(|t| {let age=now.signed_duration_since(t).num_seconds(); (age>=0).then_some(age)}),"scope":"Recorded review schedules and observation age; source URLs have not been fetched"})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_schedule_is_unknown_and_past_is_overdue() {
        let e = json!({"sources":[{"id":"a","reviewAfter":"2000-01-01T00:00:00Z"},{"id":"b"}]});
        let s = summary(Some(&e), None);
        assert_eq!(s["overdueSources"], 1);
        assert_eq!(s["unknownReviewSchedules"], 1);
        assert!(s["recordedObservationAgeSeconds"].is_null());
    }
}
