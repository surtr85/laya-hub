use std::collections::HashMap;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub matched_servers: Vec<MatchedItem>,
    pub matched_skills: Vec<MatchedItem>,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedItem {
    pub id: String,
    pub score: f32,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Serialize)]
struct LayaRequest {
    state: HashMap<String, String>,
    questions: HashMap<String, QuestionSpec>,
}

#[derive(Debug, Serialize)]
struct QuestionSpec {
    #[serde(rename = "type")]
    qtype: String,
    instructions: String,
    criteria: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct LayaResponse {
    answers: HashMap<String, Answer>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Answer {
    #[serde(default)]
    probabilities: Option<HashMap<String, f32>>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    latency_ms: f64,
}

pub struct LayaScorer {
    client: Client,
    url: String,
    threshold: f32,
    max_active_tools: usize,
}

impl LayaScorer {
    pub fn new(url: String, threshold: f32, max_active_tools: usize) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_millis(1500))
                .build()
                .unwrap_or_default(),
            url,
            threshold,
            max_active_tools,
        }
    }

    pub async fn route_task(&self, task: &str, config: &AppConfig) -> Result<RouteDecision> {
        let task_lower = task.to_lowercase();
        let mut questions: HashMap<String, QuestionSpec> = HashMap::new();

        // 1. Build server criteria
        if !config.mcp_servers.is_empty() {
            let mut server_criteria = HashMap::new();
            for (id, srv) in &config.mcp_servers {
                let desc = format!("{} (tags: {})", srv.description(), srv.tags().join(", "));
                server_criteria.insert(id.clone(), desc);
            }
            server_criteria.insert("general".to_string(), "general task without external servers / کار عمومی بدون ابزار".to_string());

            questions.insert(
                "mcp_match".to_string(),
                QuestionSpec {
                    qtype: "choice".to_string(),
                    instructions: "Which capability is best suited for prompt?".to_string(),
                    criteria: server_criteria,
                },
            );
        }

        // 2. Build skill criteria
        if !config.skills.is_empty() {
            let mut skill_criteria = HashMap::new();
            for (id, sk) in &config.skills {
                let desc = format!("{} (tags: {})", sk.description, sk.tags.join(", "));
                skill_criteria.insert(id.clone(), desc);
            }
            skill_criteria.insert("standard".to_string(), "standard execution without special skill framework / اجرای استاندارد".to_string());

            questions.insert(
                "skill_match".to_string(),
                QuestionSpec {
                    qtype: "choice".to_string(),
                    instructions: "Which specialized engineering skill guideline is relevant to prompt?".to_string(),
                    criteria: skill_criteria,
                },
            );
        }

        let mut matched_servers: Vec<MatchedItem> = Vec::new();
        let mut matched_skills: Vec<MatchedItem> = Vec::new();
        let mut latency_ms = 0.0;

        if !questions.is_empty() {
            let mut state = HashMap::new();
            state.insert("prompt".to_string(), task.to_string());

            let payload = LayaRequest { state, questions };

            if let Ok(res) = self.client.post(&self.url).json(&payload).send().await {
                if let Ok(resp) = res.json::<LayaResponse>().await {
                    latency_ms = resp.usage.map(|u| u.latency_ms).unwrap_or(0.0);

                    // Parse Server probabilities
                    if let Some(ans) = resp.answers.get("mcp_match") {
                        if let Some(probs) = &ans.probabilities {
                            let mut sorted: Vec<(&String, &f32)> = probs.iter().collect();
                            sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));

                            for (id, &prob) in sorted {
                                if id == "general" || prob < self.threshold {
                                    continue;
                                }
                                if let Some(srv) = config.mcp_servers.get(id) {
                                    matched_servers.push(MatchedItem {
                                        id: id.clone(),
                                        score: prob,
                                        description: srv.description().to_string(),
                                        content: None,
                                    });
                                }
                                if matched_servers.len() >= self.max_active_tools {
                                    break;
                                }
                            }
                        }
                    }

                    // Parse Skill probabilities
                    if let Some(ans) = resp.answers.get("skill_match") {
                        if let Some(probs) = &ans.probabilities {
                            let mut sorted: Vec<(&String, &f32)> = probs.iter().collect();
                            sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));

                            for (id, &prob) in sorted {
                                if id == "standard" || prob < self.threshold {
                                    continue;
                                }
                                if let Some(sk) = config.skills.get(id) {
                                    let content = crate::config::read_skill_content(&sk.path).ok();
                                    matched_skills.push(MatchedItem {
                                        id: id.clone(),
                                        score: prob,
                                        description: sk.description.clone(),
                                        content,
                                    });
                                }
                                if matched_skills.len() >= 2 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Fast keyword / tag-based fallback boost if Laya model was ambivalent
        for (id, srv) in &config.mcp_servers {
            if matched_servers.iter().any(|m| &m.id == id) {
                continue;
            }
            if srv.tags().iter().any(|tag| task_lower.contains(&tag.to_lowercase())) {
                matched_servers.push(MatchedItem {
                    id: id.clone(),
                    score: 0.95,
                    description: srv.description().to_string(),
                    content: None,
                });
            }
        }

        for (id, sk) in &config.skills {
            if matched_skills.iter().any(|m| &m.id == id) {
                continue;
            }
            if sk.tags.iter().any(|tag| task_lower.contains(&tag.to_lowercase())) {
                let content = crate::config::read_skill_content(&sk.path).ok();
                matched_skills.push(MatchedItem {
                    id: id.clone(),
                    score: 0.95,
                    description: sk.description.clone(),
                    content,
                });
            }
        }

        // Sort both by score descending
        matched_servers.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        matched_skills.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        if matched_servers.len() > self.max_active_tools {
            matched_servers.truncate(self.max_active_tools);
        }
        if matched_skills.len() > 2 {
            matched_skills.truncate(2);
        }

        Ok(RouteDecision {
            matched_servers,
            matched_skills,
            latency_ms,
        })
    }
}
