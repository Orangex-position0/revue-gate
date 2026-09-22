# Hybrid Retrieval

目标：优化之前实现的“向量 + FTS5”混合检索

之前的问题：权重固定，模式只有一种

修改方向：

- 支持权重参数化
- 支持三种检索模式：hybrid, vector, keyword，用户可自由切换
- 实现 CJK Bigram 中文分词，用于解决 FTS5 对中文检索不友好的问题
- 实现评分分解
- 实现 RetrievalDetail，RAG 回答要附带完整的检索细节
- 前端增加检索配置面板和检索详情展示
