# Style Exemplar

This file contains representative excerpts demonstrating the target depth and style for KB card explanations. Use these as a calibration reference — match this level of technical detail, narrative flow, and specificity.

## Example: Single Card Deep Explanation (LAPA)

> **The problem:** Training robot policies requires action-labeled data — recordings of a human teleoperating a robot where every frame is tagged with "move arm 3cm right, rotate wrist 15 degrees." This data is extremely expensive to collect. Meanwhile, the internet is full of videos showing manipulation (people cooking, assembling furniture, robots doing tasks), but none of them have action labels. LAPA bridges this gap with a three-stage pipeline.
>
> ### Stage 1: Latent Action Quantization
>
> Think of this as building an action dictionary from scratch.
>
> You take two consecutive frames from a video — before and after. Something happened between them. A hand moved, an object shifted. You don't know what the "action" was formally, but frame 2 is the result of some action applied to the state in frame 1.
>
> You train an encoder-decoder system. The encoder sees both frames and compresses "what changed" into a short discrete code — say [3, 2, 0, 1]. The decoder takes frame 1 plus that code and tries to reconstruct frame 2. If the reconstruction is good, the code must be capturing the essential action.
>
> These codes come from a VQ-VAE (Vector Quantized Variational Autoencoder). A VQ-VAE maintains a codebook — a fixed-size dictionary of learned embedding vectors. The encoder outputs a continuous vector, but it gets snapped to the nearest codebook entry via nearest-neighbor lookup. That quantized vector is what the decoder receives. The training has three loss terms: reconstruction (decode should reconstruct the input), codebook loss (move entries closer to encoder outputs), and commitment loss (encourage the encoder to commit to entries). Gradients can't flow through the discrete nearest-neighbor step, so they use a straight-through estimator — just copy the decoder's gradient directly to the encoder. To prevent codebook collapse (where most entries go unused), LAPA uses a technique called NSVQ that replaces the quantization error with a noise-scaled vector.
>
> With 8 possible values per position and 4 positions, you get 8^4 = 4,096 possible latent actions, each ending up semantically meaningful: "move down-left," "rotate right," "stay still," etc.
>
> ### Stage 2: Latent Pretraining
>
> Now the encoder becomes a labeling machine. You run it over your entire video dataset — millions of clips — and for every consecutive frame pair, extract the latent action. Now every frame has a "label," even though no human ever annotated it.
>
> With these labels, you train a large 7B Vision-Language Model to look at the current image plus a language instruction ("pick up the red cup") and predict the next latent action. This is just next-token prediction — the model outputs discrete tokens exactly like predicting the next word in a sentence.
>
> ### Stage 3: Action Finetuning
>
> You take the trained model from Stage 2 and delete the MLP head that predicted latent actions. In its place, you initialize a brand new head shaped to output real robot actions — for a 7-DOF arm, 7 dimensions (x, y, z, roll, pitch, yaw, gripper) each discretized into 256 bins.
>
> The old head's entire purpose was to give the backbone a training signal. By learning to predict latent actions over millions of videos, the backbone learned rich representations of how visual scenes, language, and physical actions relate. The head is disposable; the value is in the billions of backbone parameters underneath it.
>
> You fine-tune on a small labeled dataset (100-450 teleoperated trajectories). The vision encoder stays frozen. The language model backbone and new action head get fine-tuned. Because the conceptual gap between "predict latent actions" and "predict real end-effector deltas" is small — both represent "what physical motion to do next" — this converges fast, often in a single epoch.
>
> ### LAPA Results
>
> LAPA (Open-X) achieves 50.1% real-world success vs. OpenVLA's 43.9%, while being 30-40x more compute-efficient. When pretrained solely on human manipulation videos (no robots at all), it still outperforms OpenVLA pretrained on robot data. The same codebook entries produce similar motions across completely different robot embodiments, showing the latent actions are genuinely embodiment-agnostic.

## Example: Cross-Card Comparison Dimension (Latent Action Lineage)

> LAPA established the core idea: train a VQ-VAE on video frame pairs to learn an embodiment-agnostic action vocabulary, then use it to label unlabeled video for pretraining. GR00T N1 explicitly cites and builds on this, but diverges in implementation — it uses **continuous pre-quantized embeddings** as targets for flow matching rather than LAPA's **discrete codebook indices** for next-token prediction. It also keeps latent actions as one ingredient in a larger mixture rather than making them the central pretraining mechanism, and continues using them during post-training rather than discarding them after pretraining.
>
> Cosmos Policy sidesteps latent actions entirely. Its implicit argument is that a video diffusion model pretrained on internet video already contains the physical understanding that LAPA's latent action pretraining was designed to instill — you just need the right way to extract it.

---

**Key qualities to replicate:**
- Specific numbers inline (8^4 = 4,096; 7B parameters; 50.1% vs. 43.9%)
- Analogies before formal explanations ("Think of this as building an action dictionary from scratch")
- WHY behind each design choice, not just WHAT
- Direct comparisons name concrete implementation differences, not abstract categories
- Narrative flows as connected prose, not bullet lists
