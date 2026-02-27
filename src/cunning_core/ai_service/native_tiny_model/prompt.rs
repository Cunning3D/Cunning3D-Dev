pub struct TinyPromptBuilder;

impl TinyPromptBuilder {
    pub fn build_connection_hint(
        source_node: &str,
        source_type: &str,
        target_node: &str,
        target_type: &str,
    ) -> String {
        // System prompt strictly for tiny models (force concise output, disable chain-of-thought).
        let system_prompt = "You are the node editor's validation engine. Your task is to check connection compatibility.\
                             Output ONLY one of: 'compatible', 'incompatible', or a short suggestion (<= 10 words).\
                             /no-think\
                             ";

        format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\nSource node: '{}' (output type: {})\nTarget node: '{}' (input type: {})\nIs this connection valid?<|im_end|>\n<|im_start|>assistant\n",
            system_prompt,
            source_node, source_type,
            target_node, target_type
        )
    }

    pub fn build_general_hint(context: &str) -> String {
        let system_prompt = "You are an assistant for 3D modeling nodes.\
                             Responses must be extremely concise.\
                             /no-think\
                             ";

        format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            system_prompt,
            context
        )
    }
}
