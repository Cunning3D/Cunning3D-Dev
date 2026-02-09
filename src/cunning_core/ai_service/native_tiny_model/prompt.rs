pub struct TinyPromptBuilder;

impl TinyPromptBuilder {
    pub fn build_connection_hint(
        source_node: &str,
        source_type: &str,
        target_node: &str,
        target_type: &str,
    ) -> String {
        // System prompt strictly for Qwen 1.7B /no_think (Chinese optimized)
        let system_prompt = "你是节点编辑器的逻辑引擎。任务是检查连线兼容性。\
                             直接输出结果：'兼容'、'不兼容'，或不超过10个字的建议。
                             /no-think
                             ";

        format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n源节点: '{}' (输出类型: {})\n目标节点: '{}' (输入类型: {})\n这条连线有效吗？<|im_end|>\n<|im_start|>assistant\n",
            system_prompt,
            source_node, source_type,
            target_node, target_type
        )
    }

    pub fn build_general_hint(context: &str) -> String {
        let system_prompt = "你是3D建模节点的智能助手。\
                             回答必须极简。\
                             /no-think
                             ";

        format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            system_prompt,
            context
        )
    }
}
